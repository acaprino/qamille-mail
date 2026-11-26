import Button from '../ui/Button';
import Icon from '../ui/Icon';
import Modal, { ModalFooter } from '../ui/Modal';
import { tr } from '../lib/i18n';
import './GoogleSignInDialog.css';

export type GoogleSignInDialogProps = {
  pending: boolean;
  error: string;
  hide: () => void;
  cancel: () => void;
  retry: () => void;
  setup: () => void;
};

export default function GoogleSignInDialog({
  pending,
  error,
  hide,
  cancel,
  retry,
  setup,
}: GoogleSignInDialogProps) {
  return (
    <Modal
      size="sm"
      className="google-sign-in-dialog"
      aria-labelledby="google-sign-in-title"
      onClose={hide}
    >
      <div className={error ? 'google-dialog-heading failed' : 'google-dialog-heading'}>
        <Icon name={error ? 'alert' : 'link'} size={20} />
        <h2 id="google-sign-in-title">{tr('Accedi con Google', 'Sign in with Google')}</h2>
      </div>
      {error ? (
        <>
          <p className="google-sign-in-error" role="alert">
            {error}
          </p>
          <Button variant="text" className="google-help-link" onClick={setup}>
            {tr("Come attivare l'accesso Google", 'How to set up Google sign-in')}
          </Button>
        </>
      ) : (
        <>
          <p role="status">
            {tr("Completa l'accesso nel browser", 'Complete sign-in in your browser')}
          </p>
          <p className="google-sign-in-help">
            {tr(
              'Inserisci email e password nella pagina di Google, poi autorizza l’accesso. Se hai già effettuato l’accesso, scegli il tuo account.',
              'Enter your email and password on Google’s page, then approve access. If you are already signed in, just choose your account.',
            )}
          </p>
          <p className="google-sign-in-help">
            {tr(
              'Puoi nascondere questa finestra: il collegamento continua. L’attesa termina dopo 5 minuti.',
              'You can hide this window while sign-in continues. The request times out after 5 minutes.',
            )}
          </p>
        </>
      )}
      <ModalFooter>
        {pending && !error && (
          <Button variant="secondary" onClick={cancel}>
            {tr('Annulla accesso', 'Cancel sign-in')}
          </Button>
        )}
        <Button variant="secondary" onClick={hide}>
          {pending ? tr('Nascondi', 'Hide') : tr('Chiudi', 'Close')}
        </Button>
        {error && !pending && (
          <Button variant="primary" onClick={retry}>
            {tr('Riprova', 'Try again')}
          </Button>
        )}
      </ModalFooter>
    </Modal>
  );
}
