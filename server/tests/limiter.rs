use std::net::IpAddr;
use std::time::Duration;

use qamille_server::ServerError;
use qamille_server::limiter::{
    FailureLimiter, LoginThrottle, MAX_WAIT, THROTTLE_WINDOW, wait_for,
};
use tokio::time::Instant;

fn now() -> Instant {
    Instant::from_std(std::time::Instant::now())
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn wait_sequence_after_three_free_attempts() {
    let waits: Vec<u64> = (0..10).map(|n| wait_for(n).as_millis() as u64).collect();
    assert_eq!(waits, vec![0, 0, 0, 250, 500, 1000, 2000, 4000, 4000, 4000]);
}

#[test]
fn throttle_times_follow_the_sequence() {
    let throttle = LoginThrottle::new();
    let mut t = now();
    let expected = [0, 0, 0, 250, 500, 1000, 2000, 4000, 4000];
    for wait in expected {
        let ticket = throttle.begin("mario", t);
        let start = ticket.start();
        assert_eq!(start - t, ms(wait));
        t = start;
        ticket.failed(t);
    }
}

#[test]
fn success_resets_and_identifiers_are_normalized() {
    let throttle = LoginThrottle::new();
    let t = now();
    for _ in 0..5 {
        throttle.failed("  Mario ", t);
    }
    assert_eq!(throttle.failures("mario", t), 5);
    assert_eq!(throttle.failures("MARIO", t), 5);
    throttle.succeeded("MaRiO");
    assert_eq!(throttle.failures("mario", t), 0);
    assert_eq!(throttle.begin("mario", t).start(), t);
}

#[test]
fn failures_leave_the_window_after_fifteen_minutes() {
    let throttle = LoginThrottle::new();
    let t = now();
    for _ in 0..6 {
        throttle.failed("mario", t);
    }
    let later = t + THROTTLE_WINDOW + ms(1);
    assert_eq!(throttle.failures("mario", later), 0);
    assert_eq!(throttle.begin("mario", later).start(), later);
}

#[test]
fn fifty_simultaneous_reservations_never_wait_more_than_four_seconds() {
    for prior_failures in [3, 8] {
        let throttle = LoginThrottle::new();
        let t = now();
        for _ in 0..prior_failures {
            throttle.failed("mario", t);
        }
        let mut previous = t;
        let mut tickets = Vec::new();
        for _ in 0..50 {
            let ticket = throttle.begin("mario", t);
            let start = ticket.start();
            tickets.push(ticket);
            assert!(start - t <= MAX_WAIT, "attesa {:?}", start - t);
            assert!(start >= previous, "i turni si mettono in fila");
            previous = start;
        }
        assert_eq!(previous - t, MAX_WAIT);
    }
}

#[test]
fn full_table_does_not_reset_the_victim() {
    let throttle = LoginThrottle::with_capacity(3);
    let t = now();
    for _ in 0..6 {
        throttle.failed("u:victim", t);
    }
    throttle.failed("i:other-1", t);
    throttle.failed("i:other-1", t);
    throttle.failed("i:other-2", t + ms(1));
    assert_eq!(throttle.len(), 3);

    // Un identificativo nuovo fa posto togliendo la voce sconosciuta con meno
    // tentativi sbagliati; l'utente vittima resta con i suoi sei.
    let ticket = throttle.begin("i:newcomer", t + ms(10));
    assert!(!ticket.in_overflow());
    assert_eq!(ticket.start(), t + ms(10));
    ticket.failed(t + ms(10));
    assert_eq!(throttle.len(), 3);
    assert_eq!(throttle.failures("i:newcomer", t + ms(10)), 1);
    assert_eq!(throttle.failures("i:other-2", t + ms(10)), 0, "tolta");
    assert_eq!(throttle.failures("i:other-1", t + ms(10)), 2);
    assert_eq!(throttle.failures("u:victim", t + ms(10)), 6);

    // Fuori dalla finestra le voci vecchie si tolgono tutte.
    let later = t + THROTTLE_WINDOW + ms(20);
    assert_eq!(throttle.begin("i:newcomer-2", later).start(), later);
    assert_eq!(throttle.len(), 1);
}

#[test]
fn a_table_full_of_users_sends_newcomers_to_a_shared_counted_entry() {
    let throttle = LoginThrottle::with_capacity(2);
    let t = now();
    throttle.failed("u:anna", t);
    throttle.failed("u:mario", t);
    // Nessuna voce si può togliere: dieci tentativi in parallelo con identificativi
    // sempre nuovi finiscono nella voce comune, che conta i tentativi in corso e li
    // mette in fila come una voce normale.
    let tickets: Vec<_> = (0..10)
        .map(|n| throttle.begin(&format!("i:nuovo-{n}"), t))
        .collect();
    assert!(tickets.iter().all(|ticket| ticket.in_overflow()));
    let starts: Vec<u64> = tickets
        .iter()
        .map(|ticket| (ticket.start() - t).as_millis() as u64)
        .collect();
    assert_eq!(
        starts,
        vec![0, 0, 0, 250, 750, 1750, 3750, 4000, 4000, 4000]
    );
    assert_eq!(throttle.overflow_counts(t), (0, 10));
    for ticket in tickets {
        ticket.failed(t);
    }
    assert_eq!(throttle.overflow_counts(t), (10, 0));
    // I tentativi sbagliati restano: il prossimo aspetta subito il massimo.
    assert_eq!(throttle.begin("i:ancora", t).start() - t, MAX_WAIT);
    assert_eq!(throttle.failures("u:anna", t), 1);
    assert_eq!(throttle.len(), 2);
}

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

#[test]
fn failure_limiter_blocks_after_threshold_until_window_passes() {
    let limiter = FailureLimiter::new(3, Duration::from_secs(60));
    let t = now();
    let a = ip("203.0.113.1");
    let b = ip("203.0.113.2");
    for i in 0..3 {
        assert!(limiter.check(a, t).is_ok());
        limiter.record_failure(a, t + Duration::from_secs(i));
    }
    match limiter.check(a, t + Duration::from_secs(3)) {
        Err(ServerError::RateLimited { retry_after_s }) => assert_eq!(retry_after_s, 57),
        other => panic!("atteso rate_limited, ricevuto {other:?}"),
    }
    assert!(limiter.check(b, t).is_ok(), "le origini sono separate");
    assert!(limiter.check(a, t + Duration::from_secs(61)).is_ok());
}

#[test]
fn a_full_origin_table_never_locks_out_new_origins() {
    let limiter = FailureLimiter::with_capacity(3, Duration::from_secs(60), 3);
    let t = now();
    // Un'origine già al limite e due con un fallimento ciascuno riempiono la tabella.
    for _ in 0..3 {
        limiter.record_failure(ip("203.0.113.1"), t);
    }
    limiter.record_failure(ip("203.0.113.2"), t);
    limiter.record_failure(ip("203.0.113.3"), t + Duration::from_secs(1));
    assert_eq!(limiter.len(), 3);
    // Mille origini nuove, ognuna con un fallimento (chi inonda la tabella): nessuna
    // viene rifiutata, ognuna è contata…
    for n in 0..1000u32 {
        let origin = IpAddr::from(std::net::Ipv4Addr::from(0x0a00_0000 + n));
        let at = t + Duration::from_millis(2000 + u64::from(n));
        assert!(limiter.begin(origin, at).is_ok(), "origine {n}");
        assert!(limiter.hit(origin, at).is_ok());
        assert_eq!(limiter.failures(origin, at), 1);
    }
    assert_eq!(limiter.len(), 3);
    // …e l'origine al limite resta al limite: le voci tolte sono quelle che pesano meno.
    assert!(
        limiter
            .check(ip("203.0.113.1"), t + Duration::from_secs(3))
            .is_err()
    );
    assert_eq!(
        limiter.failures(ip("203.0.113.1"), t + Duration::from_secs(3)),
        3
    );
    // Un cliente qualsiasi entra comunque.
    let client = ip("198.51.100.77");
    let attempt = limiter.begin(client, t + Duration::from_secs(4)).unwrap();
    attempt.succeeded();
    // Dopo la finestra le voci scadute lasciano posto.
    let later = t + Duration::from_secs(120);
    limiter.record_failure(ip("203.0.113.9"), later);
    assert_eq!(limiter.failures(ip("203.0.113.9"), later), 1);
}

#[test]
fn an_attempt_whose_entry_was_evicted_still_counts_its_failure() {
    let limiter = FailureLimiter::with_capacity(5, Duration::from_secs(60), 1);
    let t = now();
    let first = limiter.begin(ip("203.0.113.1"), t).unwrap();
    // La seconda origine prende il posto della prima (che ha un solo tentativo).
    limiter.record_failure(ip("203.0.113.2"), t);
    limiter.record_failure(ip("203.0.113.2"), t);
    first.failed(t);
    assert_eq!(limiter.failures(ip("203.0.113.1"), t), 1);
}

#[test]
fn ipv6_origins_are_grouped_by_slash_64() {
    let limiter = FailureLimiter::new(3, Duration::from_secs(60));
    let t = now();
    for host in ["2001:db8:1:2::1", "2001:db8:1:2::2", "2001:db8:1:2:ffff::9"] {
        assert!(limiter.check(ip(host), t).is_ok());
        limiter.record_failure(ip(host), t);
    }
    assert_eq!(limiter.failures(ip("2001:db8:1:2::abcd"), t), 3);
    assert!(
        limiter.check(ip("2001:db8:1:2::77"), t).is_err(),
        "stessa /64"
    );
    assert!(
        limiter.check(ip("2001:db8:1:3::1"), t).is_ok(),
        "un'altra /64"
    );
    assert_eq!(limiter.len(), 1);
    // IPv4 (anche in forma mappata) resta per indirizzo.
    limiter.record_failure(ip("::ffff:203.0.113.1"), t);
    assert_eq!(limiter.failures(ip("203.0.113.1"), t), 1);
    assert_eq!(limiter.failures(ip("203.0.113.2"), t), 0);
}

#[test]
fn attempts_in_flight_count_against_the_origin_threshold() {
    let limiter = FailureLimiter::new(3, Duration::from_secs(60));
    let t = now();
    let a = ip("203.0.113.1");
    let first = limiter.begin(a, t).unwrap();
    let second = limiter.begin(a, t).unwrap();
    let third = limiter.begin(a, t).unwrap();
    // Una raffica non passa la soglia prima che le verifiche finiscano.
    assert!(matches!(
        limiter.begin(a, t),
        Err(ServerError::RateLimited { .. })
    ));
    assert_eq!(limiter.pending(a), 3);
    first.failed(t);
    second.succeeded();
    drop(third); // richiesta interrotta: esce senza contare
    assert_eq!(limiter.pending(a), 0);
    assert_eq!(limiter.failures(a, t), 1);
    assert!(limiter.begin(a, t).is_ok());
}

#[test]
fn hit_counts_every_request_and_refuses_beyond_the_limit() {
    let limiter = FailureLimiter::new(2, Duration::from_secs(3600));
    let t = now();
    let a = ip("203.0.113.1");
    assert!(limiter.hit(a, t).is_ok());
    assert!(limiter.hit(a, t).is_ok());
    assert!(matches!(
        limiter.hit(a, t),
        Err(ServerError::RateLimited {
            retry_after_s: 3600
        })
    ));
    assert!(limiter.hit(a, t + Duration::from_secs(3600)).is_ok());
}

#[test]
fn simultaneous_attempts_queue_up_through_pending_reservations() {
    let throttle = LoginThrottle::new();
    let t = now();
    // Dieci tentativi mandati insieme, nessuno ancora concluso: dal quarto in poi
    // aspettano come se i precedenti fossero già falliti, in fila.
    let tickets: Vec<_> = (0..10).map(|_| throttle.begin("u:mario", t)).collect();
    let starts: Vec<u64> = tickets
        .iter()
        .map(|ticket| (ticket.start() - t).as_millis() as u64)
        .collect();
    assert_eq!(
        starts,
        vec![0, 0, 0, 250, 750, 1750, 3750, 4000, 4000, 4000]
    );
    assert_eq!(throttle.pending("u:mario"), 10);
    let mut tickets = tickets.into_iter();
    tickets.next().unwrap().failed(t);
    tickets.next().unwrap().succeeded();
    drop(tickets);
    assert_eq!(throttle.pending("u:mario"), 0);
    assert_eq!(throttle.failures("u:mario", t), 0, "il successo azzera");
}

#[test]
fn one_account_never_holds_more_than_two_attempts_in_progress() {
    let throttle = LoginThrottle::new();
    let t = now();
    let first = throttle.try_begin("u:mario", t).unwrap();
    let second = throttle.try_begin("u:mario", t).unwrap();
    let refused = throttle.try_begin("u:mario", t).err().unwrap();
    assert!(matches!(refused, ServerError::RateLimited { retry_after_s: 4 }), "{refused:?}");
    // Another account is not touched.
    let other = throttle.try_begin("u:anna", t).unwrap();
    // A refused attempt leaves nothing behind: finishing one frees a place.
    assert_eq!(throttle.pending("u:mario"), 2);
    first.failed(t);
    let third = throttle.try_begin("u:mario", t).unwrap();
    drop((second, third, other));
    assert_eq!(throttle.pending("u:mario"), 0);
    // The plain reservation has no cap (tests and the shared overflow entry rely on it).
    let many: Vec<_> = (0..5).map(|_| throttle.begin("u:luigi", t)).collect();
    assert_eq!(throttle.pending("u:luigi"), 5);
    drop(many);
}
