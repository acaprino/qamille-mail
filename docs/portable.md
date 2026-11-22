# Qamille portable

Apri **Qamille.exe**. Al primo avvio l'archivio è vuoto: aggiungi il tuo account
da **Impostazioni → Account**.

Tutti i dati dell'app sono nella cartella **data** accanto all'eseguibile:
posta, bozze, allegati copiati, registrazioni, indice di ricerca, preferenze,
cache e credenziali. Non serve installare Qamille né copiare dati da AppData.

La **Rubrica** vive in data/contacts.json (con i permessi e la cronologia delle unioni): si sposta con la cartella data come il resto. La rubrica unificata non è un file: si ricalcola da questo e dalla posta. Le credenziali dei contatti Google sono in data/credentials.json come le altre.

Gli allegati aggiunti alle bozze vengono copiati in data e rientrano nel limite
di spazio dell'account. Le copie senza più riferimenti vengono rimosse; i file
originali scelti dall'utente restano intatti.

Per trasferire l'app, spegni prima le voci che ha scritto in Windows («Prima di
spostare o cancellare una copia», qui sotto), chiudila con **Esci** (dal menu
dell'icona nell'area di notifica: la X la lascia aperta lì) e copia **l'intera
cartella Qamille**, compresa la cartella data. Usa una cartella in cui puoi
scrivere. Per le
integrazioni con altri programmi riapplica la relativa configurazione dopo lo
spostamento. Su un altro computer o con un altro utente di Windows le credenziali
seguono «Spostare la cartella su un altro computer o utente», più sotto.

**Avvio all'accesso** (Impostazioni → Generali → Windows, spento per default):
Windows avvia questa copia nell'area di notifica. Ogni copia ha la sua voce, con
il nome della cartella e un codice della sua cartella data, che si vede e si
spegne anche in Gestione attività → App di avvio. La voce nomina l'eseguibile di
questa cartella.

**Prima di spostare o cancellare una copia**, in Impostazioni → Generali →
Windows spegni quello che la copia ha scritto nel registro di Windows del tuo
utente: l'avvio all'accesso, le **Notifiche di Windows** (accese, registrano il
nome con cui Windows le mostra) e i link mailto (**Smetti**, se avevi scelto
«Usa Qamille per i link mailto»). Ogni voce porta il codice della cartella data:
in un'altra posizione la copia ha un codice nuovo e non può più togliere le voci
di prima, che resterebbero in Windows. Poi, dalla nuova posizione, riaccendi
quello che ti serve.

Le credenziali (password e accessi di Google, degli account IMAP e del server)
sono cifrate in data/credentials.json. La chiave che le apre è in
data/credentials.key.dpapi, protetta da Windows (DPAPI) per l'utente di Windows
che usa Qamille su questo computer: copiata altrove, la cartella non basta per
leggerle. Le versioni precedenti tenevano la chiave in chiaro in
data/credentials.key: al primo avvio Qamille la fa proteggere da Windows e cancella
il file in chiaro solo dopo aver verificato che la copia protetta riapre le
credenziali.

La cartella **backups**, se presente, contiene il precedente archivio conservato
prima dell'azzeramento. Non viene caricato durante l'avvio normale.

## Spostare la cartella su un altro computer o utente

Sullo stesso computer e con lo stesso utente di Windows, spostare o copiare la
cartella non cambia niente. Su un altro computer, o aperta da un altro utente di
Windows, la cartella porta con sé posta, bozze, rubrica, impostazioni e l'elenco
degli account, ma Windows non apre la chiave delle credenziali:

- **con una frase di trasferimento** (facoltativa, in Impostazioni → Account →
  Credenziali): all'avvio Qamille la chiede, riapre le credenziali e le fa
  proteggere da Windows per il nuovo utente. La frase non è salvata da nessuna
  parte; data/credentials.key.transfer contiene una copia della chiave cifrata con
  una chiave derivata dalla frase (Argon2id, con i parametri scritti nel file).
  Chi ha la cartella e la frase ha le credenziali: scegline una lunga e non
  scriverla nella cartella. Per cambiarla o toglierla le credenziali devono
  aprirsi su questo computer.
- **senza frase**: Qamille si apre con posta e impostazioni e chiede di accedere
  di nuovo agli account (Google, IMAP e, nella Professional, il server). Al primo
  nuovo accesso, o con «Accedi di nuovo agli account», le credenziali di prima
  vanno in data/credentials-set-aside-<data e ora>: non sono cancellate, ma
  Qamille non le usa più.
- **con la frase dimenticata**: un nuovo accesso viene rifiutato finché c'è una
  frase da provare; «Non la ricordo: accedi di nuovo» mette da parte le
  credenziali nello stesso modo, e poi si accede di nuovo agli account.

Se Windows non riesce ad aprire la chiave per un altro motivo (un profilo non
caricato, un file rovinato), Qamille lo dice con il codice d'errore di Windows e
non mette da parte niente, nemmeno a un nuovo accesso: si riprova con «Riprova».
Se riprovando la chiave non si apre ancora, compare «Accedi di nuovo agli
account»: dopo una conferma mette da parte le credenziali come sopra, e se poi
Windows torna ad aprirle si possono rimettere a posto (qui sotto). In ogni caso,
se in quel momento Windows non riesce a proteggere una chiave nuova, non si
sposta niente.

**Una cartella usata a turno su due computer** (per esempio su una chiavetta):
ogni volta che passa all'altro computer, Qamille chiede la frase di trasferimento,
riapre le credenziali e le fa proteggere da Windows per quel computer. Tornata sul
primo, la chiede di nuovo. Senza frase, ogni passaggio mette da parte le
credenziali dell'altro computer al primo nuovo accesso.

**Rimettere a posto una cartella messa da parte**: chiudi Qamille con **Esci**,
sposta i file della cartella data/credentials-set-aside-<data e ora>
(credentials.json, credentials.key.dpapi e, se c'è, credentials.key.transfer) di
nuovo in data, al posto di quelli che ci sono, e riapri Qamille con l'utente di
Windows e sul computer dove erano stati creati. Altrove si aprono solo con la loro
frase di trasferimento. Le credenziali salvate dopo la messa da parte (quelle dei
nuovi accessi) si perdono: conserva prima, da qualche parte, i file che sostituisci.

## Collegamento a un server Qamille

Se hai collegato Qamille a un server (Impostazioni → Calcolo), il collegamento
è salvato nelle credenziali di data, voce `server-session`, insieme
all'identità di questo dispositivo. La password non viene mai salvata.

Copiare la cartella sullo stesso utente di Windows, o altrove con la frase di
trasferimento, copia anche il collegamento e l'identità del dispositivo:
per il server le due copie sono **un solo dispositivo**, e disattivarne una
scollega anche l'altra. Prima di dare la cartella a un'altra persona usa
**Esci da questo dispositivo** in Impostazioni → Calcolo.

L'edizione Professional (installata in `D:\Software\Qamille-Professional`, e l'installer per
utente) si collega solo al proprio server; aprendo con essa una cartella data che
veniva da un'edizione self-hosted, il collegamento al vecchio server viene tolto
all'avvio.
