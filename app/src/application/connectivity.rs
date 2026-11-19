//! Connection health behind the confirmation queue (ADR-0007 v3, decision 8).
//!
//! Every provider connection that succeeds notes its account here, and the queue
//! drains those notes to retry that account's waiting confirmations at once
//! instead of waiting for their backoff (EX-009, INV-010). The state is a set of
//! account identifiers behind one short lock: no service, storage or timer
//! dependency, so a wake costs nothing while the network is up and nothing polls
//! the network while it is down (RA-008).
//!
//! A sleeping consumer is told about a wake instead of looking for it:
//! [`subscribe`] registers its condvar signal, and `note_success` raises it
//! (INV-005: no fixed-interval re-check of this set anywhere).
//!
//! This module also owns the retry policy for provider failures, because the
//! queue is its only consumer. The texts it recognizes are produced by
//! `infrastructure::mail` and the mail libraries; that module re-exports the rule
//! as `is_retryable_connect_error` and its tests pin the contract at the source.

use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, Weak};

fn wakes() -> &'static Mutex<HashSet<u32>> {
    static WAKES: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
    WAKES.get_or_init(|| Mutex::new(HashSet::new()))
}

/// The wake-up handle of a thread that sleeps until work arrives: a flag
/// ("something changed since the last wait began") guarded by the mutex its
/// condvar waits on. Raising it under that mutex cannot be lost against a wait
/// that is about to start.
pub type WakeSignal = (Mutex<bool>, Condvar);

fn listeners() -> &'static Mutex<Vec<Weak<WakeSignal>>> {
    static LISTENERS: OnceLock<Mutex<Vec<Weak<WakeSignal>>>> = OnceLock::new();
    LISTENERS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Registers a sleeper that [`note_success`] must wake. The registry holds a
/// weak reference, so a dropped consumer is forgotten on the next call.
pub fn subscribe(signal: &Arc<WakeSignal>) {
    let mut listeners = listeners().lock().unwrap_or_else(PoisonError::into_inner);
    listeners.retain(|listener| listener.strong_count() > 0);
    listeners.push(Arc::downgrade(signal));
}

/// A connection to this account's provider succeeded, so its queued
/// confirmations can be attempted again now. Never fails and never waits on
/// anything slow: the wake set and the signals are only held for a flag flip. A
/// poisoned wake set only loses one wake, which the backoff still covers.
pub fn note_success(account_id: u32) {
    if let Ok(mut wakes) = wakes().lock() {
        wakes.insert(account_id);
    }
    // The wake is visible before any sleeper is signaled, and the set's lock is
    // released here: a woken sleeper reads it (`has_wakes`, `drain_wakes`) and
    // must find it. Signals are raised outside the registry lock, so a sleeper
    // that registers meanwhile never waits on this call.
    let live: Vec<Arc<WakeSignal>> = {
        let mut listeners = listeners().lock().unwrap_or_else(PoisonError::into_inner);
        listeners.retain(|listener| listener.strong_count() > 0);
        listeners.iter().filter_map(Weak::upgrade).collect()
    };
    for signal in live {
        let (flag, condvar) = &*signal;
        let mut raised = flag.lock().unwrap_or_else(PoisonError::into_inner);
        *raised = true;
        condvar.notify_all();
    }
}

/// Take and clear the accounts whose connectivity was restored since the last
/// call. One pass of the queue consumes each wake exactly once, so a retry that
/// fails again waits for its next deadline or for a new connection.
pub fn drain_wakes() -> HashSet<u32> {
    wakes()
        .lock()
        .map(|mut wakes| std::mem::take(&mut *wakes))
        .unwrap_or_default()
}

/// Whether any account has a pending wake, without consuming it: the queue's
/// idle fast path checks this on every pass and must not drain (and lose) wakes
/// it is about to act on.
pub fn has_wakes() -> bool {
    wakes().lock().is_ok_and(|wakes| !wakes.is_empty())
}

/// The wake set is process-wide, so the tests that touch it run one at a time: a
/// wake noted by one test must never be drained by another.
#[cfg(test)]
pub(crate) fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Refusals that the same credentials or settings would produce again (EX-010).
const REFUSALS: [&str; 20] = [
    "certificate",
    "not trusted",
    "untrusted",
    "unknownissuer",
    "hostname mismatch",
    "self signed",
    "self-signed",
    "authenticationfailed",
    "invalid credentials",
    "username and password not accepted",
    "application-specific password",
    "sign in with google again",
    "web login required",
    "imap access is disabled",
    "not enabled",
    "disabled",
    "[alert]",
    "[noperm]",
    "535",
    "534",
];
/// The provider answered a command, or the session broke while one was in
/// flight. Either way the result is known or lost, never "nothing happened":
/// a lost result after a mutation or a delivery started is never retried
/// (INV-008, RA-007).
const COMMAND_RESULTS: [&str; 12] = [
    "no response:",
    "bad response:",
    "connection lost",
    "response error",
    "network error",
    "internal client error",
    "transport has been shut down",
    "permanent error",
    "unexpected parse response",
    "unable to parse",
    "delivery result is uncertain",
    "do not resend",
];
/// Phases that provably happen before any command or any message body.
const SETUP_PREFIXES: [&str; 8] = [
    "imap connect: ",
    "imap tls: ",
    "imap server has no address",
    "imap login: ",
    "connection error",
    "tls error",
    "smtp: connection error",
    "smtp: tls error",
];
const IO_MARKERS: [&str; 7] = [
    "os error",
    "timed out",
    "timeout",
    "connection refused",
    "connection reset",
    "eof",
    "tls",
];
/// Evidence that an I/O failure happened while the connection was still being
/// established, never mid-command: name resolution, network unreachability
/// before any session exists, and a refused TCP handshake. Session-level words
/// (`socket`, `imap`, `handshake`, `ssl`, …) are deliberately absent: a bare
/// I/O error naming a socket is what an OPEN session reports when it breaks
/// while a command may already have run on the server, so it stays definitive
/// (INV-008, RA-007).
const CONNECT_CONTEXT: [&str; 7] = [
    "dns",
    "resolve",
    "lookup",
    "unreachable",
    "no route to host",
    "couldn't reach google",
    "connection refused",
];

fn contains_any(text: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| text.contains(marker))
}

/// Whether a provider failure proves that the server executed nothing, so the
/// confirmation queue may retry it (ADR-0007 v3 decision 8, INV-010).
///
/// Retryable: address resolution, TCP, TLS, a login that failed for a network
/// reason, and SMTP transport setup — all of them happen before a command or a
/// message body reaches the provider, and a `transient error` reply is the
/// provider itself asking for a later attempt. Everything else is definitive:
/// authentication and configuration refusals (EX-010), command replies, and any
/// ambiguity about a result that may already exist on the server (INV-008).
pub fn is_retryable_connect_error(message: &str) -> bool {
    let text = message.trim().to_lowercase();
    if text.is_empty() {
        return false;
    }
    if contains_any(&text, &REFUSALS) {
        return false;
    }
    if SETUP_PREFIXES.iter().any(|prefix| text.starts_with(prefix)) {
        return true;
    }
    if contains_any(&text, &COMMAND_RESULTS) {
        return false;
    }
    if text.starts_with("transient error (") || text.starts_with("smtp: transient error (") {
        return true;
    }
    contains_any(&text, &IO_MARKERS) && contains_any(&text, &CONNECT_CONTEXT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wake_is_noted_once_and_consumed_by_one_drain() {
        let _guard = test_guard();
        assert!(drain_wakes().is_empty());
        note_success(3);
        note_success(3);
        note_success(4);
        assert_eq!(drain_wakes(), HashSet::from([3, 4]));
        assert!(drain_wakes().is_empty(), "One pass consumes the wake");
    }

    #[test]
    fn has_wakes_peeks_without_consuming() {
        let _guard = test_guard();
        assert!(drain_wakes().is_empty());
        assert!(!has_wakes());
        note_success(5);
        assert!(has_wakes());
        assert!(has_wakes(), "Peeking does not consume the wake");
        assert_eq!(drain_wakes(), HashSet::from([5]));
        assert!(!has_wakes());
    }

    #[test]
    fn wakes_are_per_account() {
        let _guard = test_guard();
        note_success(7);
        let drained = drain_wakes();
        assert!(drained.contains(&7));
        assert!(!drained.contains(&8));
        assert!(drain_wakes().is_empty());
    }

    #[test]
    fn a_noted_success_raises_every_subscribed_signal_and_wakes_its_sleeper() {
        let _guard = test_guard();
        assert!(drain_wakes().is_empty());
        let signal: Arc<WakeSignal> = Arc::new((Mutex::new(false), Condvar::new()));
        let bystander: Arc<WakeSignal> = Arc::new((Mutex::new(false), Condvar::new()));
        subscribe(&signal);
        subscribe(&bystander);
        assert!(!*signal.0.lock().unwrap(), "A subscription alone raises nothing");

        // A sleeper parked on the signal with no timeout: only the notifier
        // can end this wait.
        let sleeper = {
            let signal = signal.clone();
            std::thread::spawn(move || {
                let (flag, condvar) = &*signal;
                let mut raised = flag.lock().unwrap();
                while !*raised {
                    raised = condvar.wait(raised).unwrap();
                }
                has_wakes()
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert!(!sleeper.is_finished(), "The sleeper waits for a signal");
        // A dropped consumer is skipped and never keeps the live ones unserved.
        drop(bystander);
        note_success(9);
        assert!(
            sleeper.join().unwrap(),
            "The wake is already visible when the sleeper looks"
        );
        assert_eq!(drain_wakes(), HashSet::from([9]));

        *signal.0.lock().unwrap() = false;
        note_success(9);
        assert!(*signal.0.lock().unwrap(), "A later success signals again");
        drain_wakes();
    }

    #[test]
    fn a_poisoned_lock_never_breaks_the_caller() {
        // `note_success` is called from every connection: it may not panic.
        let _guard = test_guard();
        note_success(u32::MAX);
        assert!(drain_wakes().contains(&u32::MAX));
    }

    #[test]
    fn the_policy_matches_the_texts_produced_at_the_source() {
        for retryable in [
            "IMAP connect: os error 10060",
            "IMAP connect: failed to lookup address",
            "IMAP connect: dns error: failed to lookup address information: Nodo sconosciuto. (os error 11001)",
            "IMAP connect: Connection refused (os error 10061)",
            "IMAP login: Impossibile stabilire la connessione. Una operazione di connessione non riuscita (os error 10060)",
            "Couldn't reach Google: Dns Failed: failed to lookup address information (os error 11001)",
            "Connection error: dns error: no address for smtp.gmail.com",
            "transient error (421): 4.7.0 Try again later",
        ] {
            assert!(is_retryable_connect_error(retryable), "{retryable}");
        }
        for definitive in [
            "IMAP login: [AUTHENTICATIONFAILED] Invalid credentials (Failure)",
            "IMAP TLS: The certificate chain was issued by an authority that is not trusted. (os error -2146893016)",
            "IMAP TLS: invalid peer certificate: UnknownIssuer",
            "IMAP TLS: certificate has expired",
            "IMAP TLS: hostname mismatch: certificate is not valid for imap.example.com",
            "tls error: the certificate has expired",
            "IMAP login: [ALERT] Please log in via your web browser",
            "SMTP: permanent error (535): Username and Password not accepted",
            "Connection Lost",
            "No Response: [TRYCREATE] UID MOVE failed",
            "connection reset by peer (os error 10054)",
            "network error: connection reset by peer (os error 10054)",
            "response error: incomplete response",
            "Delivery result is uncertain; do not resend automatically. Connection error: dns error",
            "Original message not found on the server: Q4. Sync mail and retry.",
            "Message not found",
            // Socket-level failures of an OPEN session and an EOF inside a
            // response: a command may already have run on the server (INV-008).
            "Socket is not connected (os error 57)",
            "A request to send or receive data was disallowed because the socket is not connected and (when sending on a datagram socket using a sendto call) no address was supplied. (os error 10057)",
            "A request to send or receive data was disallowed because the socket had already been shut down in a previous call with a previous call to shutdown. (os error 10058)",
            "An operation was attempted on something that is not a socket (os error 10038)",
            "EOF while reading the IMAP response",
            "",
        ] {
            assert!(!is_retryable_connect_error(definitive), "{definitive}");
        }
    }

    /// The mid-command candidates a fuzzy I/O fallback must never adopt: a
    /// socket that stops being connected, a socket already shut down, something
    /// that is not a socket at all, and an EOF inside an IMAP response all
    /// happen after a command may have run on the server (INV-008, RA-007).
    #[test]
    fn mid_command_socket_and_eof_failures_are_never_retryable() {
        for definitive in [
            // macOS/Linux ENOTCONN.
            "Socket is not connected (os error 57)",
            "Transport endpoint is not connected (os error 107)",
            // Windows WSAENOTCONN / WSAESHUTDOWN / WSAENOTSOCK, English and
            // Italian wording: the session was open, so the outcome is lost.
            "A request to send or receive data was disallowed because the socket is not connected and (when sending on a datagram socket using a sendto call) no address was supplied. (os error 10057)",
            "Una richiesta di invio o ricezione dati è stata negata perché il socket non è connesso e (per l'invio su un socket datagramma usando una chiamata sendto) nessun indirizzo è stato fornito. (os error 10057)",
            "A request to send or receive data was disallowed because the socket had already been shut down in a previous call with a previous call to shutdown. (os error 10058)",
            "Una richiesta di invio o ricezione dati è stata negata perché il socket è già stato chiuso in una precedente chiamata a shutdown. (os error 10058)",
            "An operation was attempted on something that is not a socket (os error 10038)",
            // The imap crate's EOF while a response was expected.
            "EOF while reading the IMAP response",
        ] {
            assert!(!is_retryable_connect_error(definitive), "{definitive}");
        }
        // Every positive pinned at the source stays retryable after the fix.
        for retryable in [
            "IMAP connect: os error 10060",
            "IMAP connect: failed to lookup address",
            "IMAP connect: dns error: failed to lookup address information: Nodo sconosciuto. (os error 11001)",
            "IMAP connect: Connection refused (os error 10061)",
            "IMAP connect: Network is unreachable (os error 10051)",
            "IMAP TLS: handshake failure",
            "IMAP TLS: connection reset by peer (os error 10054)",
            "IMAP login: Impossibile stabilire la connessione. Una operazione di connessione non riuscita (os error 10060)",
            "Couldn't reach Google: Dns Failed: failed to lookup address information (os error 11001)",
            "Connection error: dns error: no address for smtp.gmail.com",
            "SMTP: Connection error: tcp connect error: connection refused (os error 10061)",
            "transient error (421): 4.7.0 Try again later",
        ] {
            assert!(is_retryable_connect_error(retryable), "{retryable}");
        }
    }
}
