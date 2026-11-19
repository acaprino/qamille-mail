# Installare Qamille negli assistenti

In **Impostazioni → Accesso agenti → Installa negli assistenti**, scegliere il profilo già salvato e se aggiungere il collegamento MCP, la skill, oppure entrambi. Aprire l’anteprima del client desiderato e controllare le destinazioni. Il pulsante successivo applica quella specifica anteprima.

L’integrazione usa i permessi del profilo Qamille. L’installazione non cambia i limiti sugli account, sui calendari, sull’invio o sull’elaborazione AI. Non collega nuovi account email.

## Client supportati

| Assistente | Cosa prepara Qamille | Passaggio nel client |
| --- | --- | --- |
| Codex | Configurazione MCP personale e skill in `~/.agents/skills/qamille/` | Aprire una nuova sessione e verificare gli strumenti Qamille. |
| Claude Code | Configurazione MCP personale in `~/.claude.json` e skill in `~/.claude/skills/qamille/` | Aprire una nuova sessione; verificare `/mcp` e la skill. |
| Claude Desktop | Configurazione MCP locale in `%APPDATA%/Claude/claude_desktop_config.json`; skill ZIP se richiesta | Chiudere completamente e riaprire Claude; importare la skill ZIP nella relativa schermata. |
| ChatGPT Work | Plugin locale con skill e/o MCP, registrato nel catalogo personale `~/.agents/plugins/marketplace.json` | “Apri per completare” apre il plugin locale; completare l’installazione in ChatGPT desktop e iniziare una nuova chat locale. |
| Claude Cowork | Estensione `.mcpb` per il collegamento MCP e ZIP separato della skill | Importare l’estensione in Claude Desktop e la skill nella schermata delle skill; iniziare un nuovo task Cowork. |

La preparazione di un pacchetto o di una voce nel catalogo non significa che il client abbia già completato l’importazione. L’interfaccia distingue questi stati. Per Claude Cowork, copiare una skill nella cartella di Claude Code non basta: i due ambienti hanno flussi di installazione diversi.

Il riferimento di interazione è il flusso ufficiale di Spark per gli agenti AI: collegamento al servizio e skill sono componenti separati. Qamille prepara i propri file, senza dipendere da un account o da un’installazione Spark.

## Chi scrive le risposte dell'assistente di Qamille

Risposte, riassunti, suggerimenti di scrittura, classificazione e punti d'azione delle riunioni possono passare da due strade, a scelta in Impostazioni → Assistente:

- **Server Qamille**: usa il server collegato, senza chiavi da incollare. Compare per primo quando il server ha un assistente ed è il predefinito al collegamento (se non avevi già scelto un altro provider). Nel passo «Prima di collegarti» Qamille dice se il server inoltra le richieste a un servizio esterno e a quale; se l'amministratore cambia provider, l'assistente si ferma finché non confermi di nuovo.
- **Usa una tua chiave**: Anthropic, un endpoint compatibile con OpenAI o Ollama, chiamati direttamente dall'app come prima.

## Anteprima e conservazione dei file

L’anteprima mostra i file da creare o aggiornare. Il server verifica nuovamente eseguibile, profilo e configurazioni prima di applicarla: se sono cambiati, richiede una nuova anteprima. Le configurazioni degli altri server e le impostazioni del client vengono conservate. Una voce Qamille estranea o modificata manualmente produce un conflitto esplicito.

Prima di aggiornare un file esistente viene creata una copia di sicurezza. JSON o TOML non validi vengono segnalati conservando il file originale. Le installazioni ripetute con lo stesso contenuto non aggiungono server duplicati.

Su Windows un file in uso dal client viene segnalato prima della modifica: chiudere il client e generare una nuova anteprima. Se un altro programma crea una nuova versione durante la pubblicazione, quella versione viene conservata. I file rimasti senza destinazione dopo un’interruzione vengono recuperati dal backup verificato alla successiva apertura dello stato o dell’anteprima. Aggiungere la sola skill o il solo collegamento MCP a ChatGPT conserva il componente già preparato.

Una copia dell’eseguibile viene conservata nella cartella dati di Qamille, sotto `integrations/bin/<hash>/`. I client ricevono un percorso stabile e la variabile `QAMILLE_DATA`, in modo da utilizzare lo stesso archivio e gli stessi profili dell’app. Vengono rispettate `CODEX_HOME` per la configurazione Codex e `CLAUDE_CONFIG_DIR` per la configurazione e le skill di Claude Code.

I pacchetti generati si trovano sotto `integrations/exports/`. Contengono riferimenti e impostazioni per questo PC; non includono password o token. L’estensione nativa richiede Windows e i runtime già necessari a Qamille. Questi pacchetti non pubblicano il server su Internet e non creano un collegamento remoto per i GPT sul web.

## Skill inclusa

La skill `qamille` spiega scoperta di account e capacità, ricerca con copertura, revisioni delle bozze, idempotenza, stati dell’outbox, calendari e meeting salvati. Non spedisce messaggi da sola e non sostituisce il collegamento MCP.

Per visualizzarla anche da un terminale:

```powershell
qamille-mcp.exe --skill
```

## Riferimenti dei formati

- [Codex e ChatGPT desktop: configurazione MCP](https://learn.chatgpt.com/docs/extend/mcp?surface=cli) e [percorsi delle skill](https://learn.chatgpt.com/docs/build-skills).
- [ChatGPT Work: plugin locali e cataloghi personali](https://developers.openai.com/plugins/build/plugins) e [apertura del plugin locale](https://learn.chatgpt.com/docs/app/commands).
- [Claude Code: MCP](https://code.claude.com/docs/en/mcp) e [skill](https://code.claude.com/docs/en/skills).
- [Claude Cowork: supporto dei plugin](https://claude.com/docs/plugins/platform-support) e [importazione delle skill](https://support.claude.com/en/articles/12512198-how-to-create-custom-skills).
- [Manifest delle estensioni MCPB](https://github.com/modelcontextprotocol/mcpb/blob/main/MANIFEST.md).

## Verifiche dell’implementazione

Sono passati 13 test Rust specifici: conservazione delle configurazioni JSON/TOML, idempotenza, anteprime obsolete, conflitti, profili, percorsi personalizzati, stato dei file rimossi e formati dei pacchetti. Log: `checks/assistant-install-tests.log`.

La skill ha superato il validatore del formato. La build web è riuscita senza errori o avvisi; due scenari browser verificano anteprima, selezione dei componenti e importazione Cowork simulata. Altri tre scenari confermano bozze Ccn, calendario e impostazioni MCP esistenti.

Entrambi gli eseguibili Windows sono stati compilati e aggiornati in `artifacts/mcp/`. Il controllo dell’avvio verifica la skill incorporata e una connessione MCP con un profilo chiamato `skill`, per evitare che il nome del profilo venga interpretato come un comando. Log: `checks/assistant-startup-smoke.log`. Sono passati anche i controlli del binario desktop su invio sintetico idempotente, permessi, revoca e processo di estrazione: `checks/assistant-install-binary-smoke.log`.

Le prove usano cartelle temporanee e comandi del client simulati. Non installano nulla nei profili reali di Codex, Claude o ChatGPT e non contattano account email.

Le successive correzioni e le verifiche complete sono descritte in [Correzioni delle review](mcp-review-fixes.md).
