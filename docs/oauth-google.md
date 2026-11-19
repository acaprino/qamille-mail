# Accesso Google in Qamille

L'utente seleziona **Accedi con Google**, completa l'accesso e il consenso sulla pagina ufficiale Google e torna a Qamille. Email e password vengono inserite soltanto su Google. La finestra di attesa nell'app mostra lo stato; la pagina Google si apre nel browser di sistema perché Google non consente il login OAuth nei browser incorporati controllati dall'app.

Il progetto OAuth appartiene allo sviluppatore di Qamille. Si configura una volta per la distribuzione dell'app: ciascun utente non deve creare il proprio progetto. In una build personale ancora priva della configurazione dell'app, è possibile configurare un proprio client nelle opzioni sviluppatore.

## Creare il progetto una volta

1. Aprire [Google Cloud Console](https://console.cloud.google.com/) con l'account che gestisce Qamille, creare un progetto e selezionarlo.
2. Aprire [Google Auth Platform](https://console.cloud.google.com/auth/overview). Configurare il nome dell'app e i contatti in **Branding**; scegliere il pubblico in **Audience**. Per una prova personale in modalità **Testing**, aggiungere l'indirizzo Google da collegare fra gli utenti di test. Non scegliere **Internal** per un'app destinata a utenti esterni alla propria organizzazione Workspace.
3. In **Data Access**, configurare gli scope di posta `https://mail.google.com/` e `https://www.googleapis.com/auth/userinfo.email`. Qamille richiede i consensi di calendario e rubrica separatamente. Nello stesso progetto, abilitare i servizi usati dalle funzioni collegate:
   - [Gmail API](https://console.cloud.google.com/apis/library/gmail.googleapis.com): bozze cloud.
   - [Google Calendar API](https://console.cloud.google.com/apis/library/calendar-json.googleapis.com): calendario.
   - [People API](https://console.cloud.google.com/apis/library/people.googleapis.com): rubrica Google (contatti e altri contatti).

   Per la rubrica, Qamille chiede il consenso ai contatti separatamente (lettura al collegamento della rubrica, scrittura solo alla prima modifica che tocca Google) e lo conserva a parte, come per il calendario. In **Data Access** aggiungere `https://www.googleapis.com/auth/contacts.readonly`, `https://www.googleapis.com/auth/contacts.other.readonly` e `https://www.googleapis.com/auth/contacts`. La classificazione di questi scope per la verifica Google non è stata controllata: va verificata in Cloud Console prima di una distribuzione pubblica.
4. In [Clients](https://console.cloud.google.com/auth/clients), scegliere **Create client**, tipo **Desktop app**, con un nome riconoscibile. Conservare l'ID e il secret assegnati a quel client. Il flusso usa un indirizzo di ritorno locale con porta casuale: non creare un client Web e non riutilizzare credenziali di altre applicazioni.
5. Per la prova personale, aprire **Impostazioni → Account → Opzioni sviluppatore**, inserire ID e secret del client Desktop e salvare. Usare quindi **Accedi con Google** e accedere con uno degli utenti di test. La guida **Come attivare l'accesso Google** resta disponibile direttamente nella sezione Account.

I nomi delle sezioni della console possono cambiare. La [guida ufficiale OAuth per app desktop](https://developers.google.com/identity/protocols/oauth2/native-app) documenta la registrazione, PKCE e il ritorno locale; le [policy OAuth Google](https://developers.google.com/identity/protocols/oauth2/policies) spiegano i requisiti dell'app distribuita.

## Preparare una build per gli utenti

Prima di compilare, lo sviluppatore imposta **insieme** `QAMILLE_GOOGLE_CLIENT_ID` e `QAMILLE_GOOGLE_CLIENT_SECRET` per il client Desktop di Qamille nell'ambiente del processo di compilazione. Le variabili vengono acquisite nel backend Rust con `option_env!`; `build-release.bat` eredita quelle già impostate. L'eseguibile risultante offre l'accesso senza chiedere agli utenti di inserire ID o secret.

Per sviluppo locale, un ID `QAMILLE_GOOGLE_CLIENT_ID` non vuoto nell'ambiente dell'app sostituisce la coppia compilata, usando soltanto `QAMILLE_GOOGLE_CLIENT_SECRET` dello stesso ambiente. Un secret di runtime senza ID viene ignorato: non viene associato automaticamente a un ID diverso.

Le impostazioni personali di un client già esistente restano supportate. Un ID personalizzato esplicito prevale sul client predefinito. Il secret viene cercato per l'ID effettivo dell'account: prima nella coppia di runtime, poi nella coppia compilata, infine nelle impostazioni personali. Il client compilato rimane quindi disponibile per rinnovare gli account già collegati anche quando un altro client di runtime è selezionato per i nuovi accessi. Anche il consenso del calendario usa questa scelta per ID.

Se non esistono né configurazione dell'app né ID personale, l'accesso si ferma con il messaggio: “Google sign-in is not configured in this version of Qamille. Contact the app developer.” Non viene aperto un tentativo di login con credenziali inventate. Le credenziali di applicazioni terze non sono riutilizzate.

## Dalla prova personale alla distribuzione

Un progetto in **Testing** autorizza solo gli utenti di test configurati. Per gli scope usati dalla posta, i refresh token in modalità Testing possono scadere dopo sette giorni: il successivo accesso richiede nuovamente il consenso. Vedere [pubblico e utenti di test](https://support.google.com/cloud/answer/15549945) e [scadenza dei refresh token](https://developers.google.com/identity/protocols/oauth2/#expiration). La distribuzione pubblica richiede la configurazione del pubblico e l'eventuale verifica Google per gli scope sensibili o con restrizioni; il consenso completo di Gmail è uno scope con restrizioni. Pubblicare il progetto non completa automaticamente questa verifica. Consultare la [guida agli scope Gmail](https://developers.google.com/workspace/gmail/api/auth/scopes) e la [guida di verifica Google](https://support.google.com/cloud/answer/13464321).

Un client Desktop è un client pubblico: il secret distribuito con l'eseguibile non è una password privata dell'utente. La protezione del codice di autorizzazione usa PKCE e `state`. Qamille conserva i refresh token nel vault cifrato portable, separatamente dai dati della posta, dentro `data/` accanto all'eseguibile. Anche la chiave del vault risiede in questa cartella: chi ne possiede una copia completa può usare le credenziali salvate. L'alternativa manuale esistente usa una [password per app Google](https://support.google.com/accounts/answer/185833), non la normale password Google.

L'accesso reale non è verificabile senza creare il progetto e completare personalmente il consenso Google. I test automatici coprono configurazione, separazione delle credenziali, callback e protezione da tentativi simultanei usando dati sintetici.
