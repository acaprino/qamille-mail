import { useState, type MouseEvent } from 'react';
import Button from '../ds/primitives/Button';
import Icon from '../ds/primitives/Icon';
import Modal from '../ds/primitives/Modal';
import { openExternal } from '../lib/external-link';
import { tr } from '../lib/i18n';
import './GoogleSetupGuide.css';

export type GoogleSetupGuideProps = { close: () => void; configure: () => void };

export default function GoogleSetupGuide({ close, configure }: GoogleSetupGuideProps) {
  const [linkError, setLinkError] = useState('');

  function openLink(event: MouseEvent<HTMLAnchorElement>) {
    setLinkError('');
    return openExternal(event, setLinkError);
  }

  return (
    <Modal className="google-setup-guide" aria-labelledby="google-setup-title" onClose={close}>
      <div className="google-setup-heading">
        <div>
          <span className="page-kicker">
            {tr('Configurazione iniziale · circa 15 minuti', 'One-time setup · about 15 minutes')}
          </span>
          <h2 id="google-setup-title">
            {tr("Come attivare l'accesso Google", 'How to set up Google sign-in')}
          </h2>
        </div>
        <Button
          variant="icon"
          aria-label={tr('Chiudi la guida', 'Close guide')}
          tip={tr('Chiudi la guida', 'Close guide')}
          onClick={close}
        >
          <Icon name="close" />
        </Button>
      </div>
      <p className="google-setup-intro">
        {tr(
          'Google deve riconoscere Qamille prima di consentire il collegamento. Chi prepara l’app esegue questi passaggi una sola volta; chi la usa accede con il proprio account Google.',
          'Google needs to recognize Qamille before allowing sign-in. The person setting up the app completes these steps once; users then sign in with their own Google account.',
        )}
      </p>
      <ol>
        <li>
          <strong>{tr('Crea il progetto Qamille', 'Create the Qamille project')}</strong>
          <p>
            {tr(
              'Accedi alla console con il tuo account Google e crea un progetto chiamato Qamille. Selezionalo prima dei passaggi successivi.',
              'Sign in to the console with your Google account and create a project named Qamille. Select it before continuing.',
            )}
          </p>
          <a
            href="https://console.cloud.google.com/projectcreate"
            target="_blank"
            rel="noopener noreferrer"
            onClick={(event) => void openLink(event)}
          >
            {tr('Crea progetto su Google Cloud', 'Create a Google Cloud project')}{' '}
            <span aria-hidden="true">↗</span>
          </a>
        </li>
        <li>
          <strong>{tr('Abilita i servizi Google', 'Enable Google services')}</strong>
          <p>
            {tr(
              'Apri Gmail API e premi Abilita. Abilita anche Google Calendar API se vuoi collegare il calendario e People API se vuoi collegare la rubrica Google.',
              'Open Gmail API and select Enable. Also enable Google Calendar API if you want to connect your calendar, and People API if you want to connect Google Contacts.',
            )}
          </p>
          <div className="google-setup-links">
            <a
              href="https://console.cloud.google.com/apis/library/gmail.googleapis.com"
              target="_blank"
              rel="noopener noreferrer"
              onClick={(event) => void openLink(event)}
            >
              Gmail API <span aria-hidden="true">↗</span>
            </a>
            <a
              href="https://console.cloud.google.com/apis/library/calendar-json.googleapis.com"
              target="_blank"
              rel="noopener noreferrer"
              onClick={(event) => void openLink(event)}
            >
              Google Calendar API <span aria-hidden="true">↗</span>
            </a>
            <a
              href="https://console.cloud.google.com/apis/library/people.googleapis.com"
              target="_blank"
              rel="noopener noreferrer"
              onClick={(event) => void openLink(event)}
            >
              People API <span aria-hidden="true">↗</span>
            </a>
          </div>
        </li>
        <li>
          <strong>{tr('Prepara la schermata di consenso', 'Set up the consent screen')}</strong>
          <p>
            {tr(
              'In Google Auth Platform → Branding, premi Inizia e inserisci Qamille come nome, la tua email di assistenza e quella di contatto. Scegli il pubblico Esterno. In Pubblico → Utenti di test aggiungi l’indirizzo Google con cui accederai.',
              'In Google Auth Platform → Branding, select Get started and enter Qamille as the name, your support email and contact email. Choose an External audience. Under Audience → Test users, add the Google address you will sign in with.',
            )}
          </p>
          <div className="google-setup-links">
            <a
              href="https://console.cloud.google.com/auth/branding"
              target="_blank"
              rel="noopener noreferrer"
              onClick={(event) => void openLink(event)}
            >
              Branding <span aria-hidden="true">↗</span>
            </a>
            <a
              href="https://console.cloud.google.com/auth/audience"
              target="_blank"
              rel="noopener noreferrer"
              onClick={(event) => void openLink(event)}
            >
              {tr('Pubblico e utenti di test', 'Audience and test users')}{' '}
              <span aria-hidden="true">↗</span>
            </a>
          </div>
        </li>
        <li>
          <strong>
            {tr(
              'Aggiungi i permessi e crea il client desktop',
              'Add permissions and create the desktop client',
            )}
          </strong>
          <p>
            {tr(
              'In Accesso ai dati aggiungi i due ambiti qui sotto. Poi apri Client → Crea client e scegli App desktop, con nome Qamille. Per questo tipo di client non devi inserire un indirizzo di reindirizzamento.',
              'Under Data Access, add the two scopes below. Then open Clients → Create client and choose Desktop app, named Qamille. You do not need to enter a redirect address for this client type.',
            )}
          </p>
          <div className="google-setup-scopes">
            <code>https://mail.google.com/</code>
            <code>https://www.googleapis.com/auth/userinfo.email</code>
          </div>
          <div className="google-setup-links">
            <a
              href="https://console.cloud.google.com/auth/scopes"
              target="_blank"
              rel="noopener noreferrer"
              onClick={(event) => void openLink(event)}
            >
              {tr('Accesso ai dati', 'Data Access')} <span aria-hidden="true">↗</span>
            </a>
            <a
              href="https://console.cloud.google.com/auth/clients"
              target="_blank"
              rel="noopener noreferrer"
              onClick={(event) => void openLink(event)}
            >
              {tr('Crea client OAuth', 'Create OAuth client')} <span aria-hidden="true">↗</span>
            </a>
          </div>
        </li>
        <li>
          <strong>{tr('Collega Qamille', 'Connect Qamille')}</strong>
          <p>
            {tr(
              'Copia l’ID client e il client secret mostrati da Google. Premi Inserisci credenziali qui sotto, incollali nei due campi e salva la configurazione. Infine premi Accedi con Google: email e password si inseriscono nella pagina di Google.',
              'Copy the client ID and client secret shown by Google. Select Enter credentials below, paste them into the two fields and save the configuration. Finally select Sign in with Google: enter your email and password on Google’s page.',
            )}
          </p>
        </li>
      </ol>
      <p className="google-setup-note">
        {tr(
          'In modalità Testing possono accedere solo gli utenti di test e il consenso per la posta scade dopo 7 giorni. Per distribuire l’app al pubblico occorre completare la verifica richiesta da Google.',
          'In Testing mode, only test users can sign in and mail authorization expires after 7 days. Public distribution requires completing Google’s verification process.',
        )}
      </p>
      <a
        href="https://developers.google.com/identity/protocols/oauth2/native-app"
        target="_blank"
        rel="noopener noreferrer"
        onClick={(event) => void openLink(event)}
      >
        {tr('Leggi la guida ufficiale Google', 'Read Google’s official guide')}{' '}
        <span aria-hidden="true">↗</span>
      </a>
      {linkError && (
        <p className="google-setup-error" role="alert">
          {linkError}
        </p>
      )}
      <div className="google-setup-actions">
        <Button variant="secondary" onClick={close}>
          {tr('Chiudi', 'Close')}
        </Button>
        <Button variant="primary" onClick={configure}>
          {tr('Inserisci credenziali', 'Enter credentials')}
        </Button>
      </div>
    </Modal>
  );
}
