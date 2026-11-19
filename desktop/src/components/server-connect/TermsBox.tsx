// «I have read the Terms and the privacy notice» (wave 9): shown only when the server declares its
// documents in `GET /v1/info`. The texts are the owner's and live on the server's site: this box
// only opens their addresses in the browser (`https://` only, INV-26) and tells the sign-up that
// the person agreed. The version that goes back to the server is the one the server declared.
import Checkbox from '../../ds/primitives/Checkbox';
import { call, errorText } from '../../lib/api';
import { tr } from '../../lib/i18n';
import type { LegalInfo } from '../../lib/server-connect';
import { legalAddress } from '../../lib/subscription-usage';

export type TermsBoxProps = {
  legal: LegalInfo;
  checked: boolean;
  onChange: (checked: boolean) => void;
  onError: (message: string) => void;
};

export default function TermsBox({ legal, checked, onChange, onError }: TermsBoxProps) {
  const terms = legalAddress(legal.terms_url);
  const privacy = legalAddress(legal.privacy_url);
  const open = (url: string) => (event: { preventDefault: () => void }) => {
    event.preventDefault();
    call('open_link', { url }).catch((error) => onError(errorText(error)));
  };
  return (
    <div className="server-terms">
      <Checkbox checked={checked} onChange={onChange}>
        {tr("Ho letto i Termini e l'informativa", 'I have read the Terms and the privacy notice')}
      </Checkbox>
      {(terms || privacy) && (
        <p className="field-help">
          {terms && (
            <a href={terms} onClick={open(terms)}>
              {tr('Leggi i Termini', 'Read the Terms')}
            </a>
          )}
          {terms && privacy && ' · '}
          {privacy && (
            <a href={privacy} onClick={open(privacy)}>
              {tr("Leggi l'informativa", 'Read the privacy notice')}
            </a>
          )}
        </p>
      )}
    </div>
  );
}
