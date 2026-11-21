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

Per trasferire l'app, chiudila con **Esci** (dal menu dell'icona nell'area di
notifica: la X la lascia aperta lì) e copia **l'intera cartella Qamille**,
compresa la cartella data. Usa una cartella in cui puoi scrivere. Per le
integrazioni con altri programmi riapplica la relativa configurazione dopo lo
spostamento.

**Avvio all'accesso** (Impostazioni → Generali → Windows, spento per default):
Windows avvia questa copia nell'area di notifica. Ogni copia ha la sua voce, con
il nome della cartella e un codice della sua cartella data, che si vede e si
spegne anche in Gestione attività → App di avvio. La voce nomina l'eseguibile di
questa cartella: prima di spostare o cancellare la copia spegni l'avvio
all'accesso, poi riaccendilo dalla nuova posizione.

Le credenziali sono cifrate in data/credentials.json; data/credentials.key è la
chiave necessaria per riaprirle anche su un altro computer. Chi possiede entrambi
i file può usare le credenziali salvate: mantieni privata la cartella completa.

La cartella **backups**, se presente, contiene il precedente archivio conservato
prima dell'azzeramento. Non viene caricato durante l'avvio normale.

## Collegamento a un server Qamille

Se hai collegato Qamille a un server (Impostazioni → Calcolo), il collegamento
è salvato nelle credenziali di data, voce `server-session`, insieme
all'identità di questo dispositivo. La password non viene mai salvata.

Copiare la cartella copia anche il collegamento e l'identità del dispositivo:
per il server le due copie sono **un solo dispositivo**, e disattivarne una
scollega anche l'altra. Prima di dare la cartella a un'altra persona usa
**Esci da questo dispositivo** in Impostazioni → Calcolo.

L'edizione Professional (installata in `D:\Software\Qamille-Professional`, e l'installer per
utente) si collega solo al proprio server; aprendo con essa una cartella data che
veniva da un'edizione self-hosted, il collegamento al vecchio server viene tolto
all'avvio.
