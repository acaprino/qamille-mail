// The search under the top bar: the chips of the applied filters, the operators, the errors, the recovery and the server
// search. What narrows a search opens from the search field as one panel (SearchPanel): quick filters, recent searches,
// people, and the form of every filter. The query is the one source of truth.
import { useEffect, type KeyboardEvent } from 'react';
import Button from '../ui/Button';
import Tooltip from '../ui/Tooltip';
import Icon from '../ui/Icon';
import { usePopup } from '../ui/motion';
import { backendText, isItalian, tr } from '../lib/i18n';
import { regionalLocale, regionalTimeZone } from '../lib/regional';
import {
  aroundRange,
  calendarLabel,
  canonicalSearchQuery,
  filterValue,
  quoteSearch,
  recoverySuggestions,
  rememberSearch,
  removeSearchChip,
  searchChips,
  searchErrorMessage,
} from '../lib/search';
import { useReactive } from '../lib/store';
import type { MailPage, Workspace } from '../lib/types';
import { imeComposing } from '../lib/ime';
import SearchPanel from './SearchPanel';

export type SearchBarProps = {
  /** The filters panel (the five filters and their forms) is open, under the search field. */
  filtersOpen: boolean;
  onclosefilters: () => void;
  onopenfilters: () => void;
  query: string;
  workspace: Workspace;
  accountId: number | null;
  page: MailPage;
  loading: boolean;
  error: string;
  displayedQuery: string;
  sort: string;
  remoteBusy: boolean;
  /** The server has more results for this search. */
  remoteMore: boolean;
  canUndo: boolean;
  onquery: (query: string, recovery?: boolean) => void;
  onsort: (sort: string) => void;
  onremote: () => void;
  onremotemore: () => void;
  oncancel: () => void;
  onundo: () => void;
  onallaccounts: () => void;
  focusSearch: () => void;
};

const FORM_OPERATORS = ['from', 'to', 'cc', 'subject', 'body', 'account', 'label', 'category', 'around', 'on', 'between', 'filename', 'type', 'has', 'in', 'is'];
const t = tr;

export default function SearchBar({
  filtersOpen,
  onclosefilters,
  onopenfilters,
  query,
  workspace,
  accountId,
  page,
  loading,
  error,
  displayedQuery,
  sort,
  remoteBusy,
  remoteMore,
  canUndo,
  onquery,
  onsort,
  onremote,
  onremotemore,
  oncancel,
  onundo,
  onallaccounts,
  focusSearch,
}: SearchBarProps) {
  const s = useReactive(() => ({
    recovering: false,
    copied: false,
    showOperators: false,
    advancedFirst: false,
  }));
  const locale = workspace.settings.locale || regionalLocale();
  const zone = page.search?.timezone || regionalTimeZone();
  // Chip and error wording follow the interface language. Dates still follow the regional format.
  const wordingLocale = isItalian() ? 'it-IT' : 'en-US';
  const accounts = workspace.accounts.map((item) => ({ value: item.email, label: `${item.name} · ${item.email}` }));
  const chips = searchChips(query, wordingLocale);
  const around = aroundRange(query, locale);
  const alternatives = recoverySuggestions(query, locale, accountId !== null, isItalian() ? 'it-IT' : 'en-US');
  const scope = filterValue(query, 'in');
  const account = workspace.accounts.find((item) => item.id === accountId);
  const coverageIncomplete = !!page.search && page.search.coverage !== 'complete';
  // The panel opens from the search field: it comes in from the side of it.
  const popup = usePopup(() => document.querySelector('.global-search'));

  useEffect(() => {
    if (!filtersOpen) s.advancedFirst = false;
  }, [filtersOpen]);
  // A press outside the panel (and outside the search field, or a list the panel opened) closes it.
  useEffect(() => {
    if (!filtersOpen) return;
    const away = (event: MouseEvent) => {
      const target = event.target instanceof Element ? event.target : null;
      if (target?.closest('#search-filters-popover, .global-search, [data-slot="select-content"], [data-part="content"]')) return;
      onclosefilters();
    };
    document.addEventListener('mousedown', away);
    return () => document.removeEventListener('mousedown', away);
    // onclosefilters only writes a flag.
  }, [filtersOpen]);

  function editChip(index: number) {
    const chip = chips[index];
    // A filter of the form opens the form; a word or another operator stays in the search field.
    if (FORM_OPERATORS.includes(chip.operator)) {
      s.advancedFirst = true;
      onopenfilters();
    } else focusSearch();
  }
  function recover(value: string) {
    onquery(value, true);
    s.recovering = false;
  }
  async function copyQuery() {
    try {
      await navigator.clipboard.writeText(canonicalSearchQuery(query, locale));
      s.copied = true;
      setTimeout(() => (s.copied = false), 1800);
    } catch {
      focusSearch();
    }
  }

  if (!filtersOpen && !query.trim() && !error) return null;
  return (
    <section className="search-workbench" aria-label={t('Filtri di ricerca', 'Search filters')}>
      {filtersOpen && (
        <div
          ref={popup}
          id="search-filters-popover"
          className="search-filter-popover"
          role="group"
          aria-label={t('Filtri di ricerca', 'Search filters')}
          onKeyDown={(event: KeyboardEvent) => {
            if (event.key === 'Escape' && !imeComposing(event.nativeEvent)) {
              event.stopPropagation();
              onclosefilters();
              focusSearch();
            }
          }}
        >
          <SearchPanel
            query={query}
            locale={locale}
            zone={zone}
            accountId={accountId}
            accounts={accounts}
            labels={workspace.labels}
            sort={sort}
            closest={!!around}
            advanced={s.advancedFirst}
            onapply={(value, keepOpen) => {
              onquery(value);
              if (keepOpen) return;
              rememberSearch(value);
              onclosefilters();
              focusSearch();
            }}
            onsort={onsort}
          />
        </div>
      )}

      {!!query.trim() && (
        <>
          <div className="search-applied-filters" aria-label={t('Filtri applicati', 'Applied filters')}>
            {chips.map((chip, index) => (
              <span key={`${index}:${chip.text}`} className="search-chip">
                <button onClick={() => editChip(index)} title={t('Modifica filtro', 'Edit filter')}>
                  {chip.text}
                </button>
                <Tooltip text={`${t('Rimuovi', 'Remove')} ${chip.text}`}>
                  <button
                    className="search-chip-remove"
                    aria-label={`${t('Rimuovi', 'Remove')} ${chip.text}`}
                    onClick={() => onquery(removeSearchChip(query, index))}
                  >
                    <Icon name="close" size={13} />
                  </button>
                </Tooltip>
              </span>
            ))}{' '}
            <Button variant="text" onClick={() => onquery('')}>
              {t('Cancella ricerca', 'Clear search')}
            </Button>{' '}
            <Button variant="text" onClick={() => (s.showOperators = !s.showOperators)}>
              {s.showOperators ? t('Nascondi operatori', 'Hide operators') : t('Mostra operatori', 'Show operators')}
            </Button>
          </div>
          {s.showOperators && (
            <div className="search-query-code">
              <code>{canonicalSearchQuery(query, locale)}</code>
              <Button variant="secondary" onClick={copyQuery}>
                {s.copied ? t('Copiata', 'Copied') : t('Copia ricerca', 'Copy search')}
              </Button>
            </div>
          )}
          <p className="search-scope">
            {t('Ambito', 'Scope')}: {filterValue(query, 'account') || account?.name || t('tutti gli account', 'all accounts')}{' '}
            · {scope || t('tutta la posta, esclusi Spam e Cestino', 'all mail, except Spam and Trash')} · {zone}
            {around && (
              <span>
                {calendarLabel(around.start, locale)} — {calendarLabel(around.end, locale)}
              </span>
            )}
          </p>
        </>
      )}

      {error && (
        <div className="search-error" role="alert">
          <strong>{t('Correggi la ricerca', 'Check your search')}</strong>
          <span>{searchErrorMessage(wordingLocale, error)}</span>
          <small>
            {t('Sono ancora visibili i risultati dell’ultima ricerca valida', 'Showing the last valid search results')}:{' '}
            {displayedQuery || t('elenco precedente', 'previous message list')}
          </small>
          <Button variant="secondary" onClick={() => onquery(quoteSearch(query))}>
            {t('Cerca come testo', 'Search as text')}
          </Button>
        </div>
      )}
      {loading && !!query && (
        <p className="search-progress" role="status">
          {t('Sto cercando…', 'Searching…')}
        </p>
      )}

      {!!query.trim() && !error && (
        <div className="search-recovery">
          {coverageIncomplete && (
            <p className="search-coverage" role="status">
              <strong>{t('Risultati parziali', 'Partial results')}</strong> ·{' '}
              {backendText(
                page.search?.notice ||
                  t('La copertura della posta non è ancora verificata.', 'Mail coverage has not been verified yet.'),
              )}
            </p>
          )}
          {!coverageIncomplete && page.search?.notice && (
            <p className="search-coverage" role="status">
              {backendText(page.search.notice)}
            </p>
          )}
          <div className="search-recovery-actions">
            {coverageIncomplete && page.search?.remote_available && !remoteMore && (
              <Button variant="secondary" disabled={remoteBusy || loading} onClick={onremote}>
                {remoteBusy ? t('Ricerca sul server…', 'Searching server…') : t('Cerca sul server', 'Search server')}
              </Button>
            )}
            {remoteMore && page.search?.remote_available && (
              <Button variant="secondary" disabled={remoteBusy || loading} onClick={onremotemore}>
                {remoteBusy
                  ? t('Ricerca sul server…', 'Searching server…')
                  : t('Altri risultati dal server', 'More results from the server')}
              </Button>
            )}
            {remoteBusy && (
              <Button variant="text" onClick={oncancel}>
                {t('Annulla ricerca sul server', 'Cancel server search')}
              </Button>
            )}{' '}
            <Button variant="text" aria-expanded={s.recovering} onClick={() => (s.recovering = !s.recovering)}>
              {t('Non la trovo', 'I can’t find it')}
            </Button>{' '}
            {canUndo && (
              <Button variant="text" onClick={onundo}>
                {t('Annulla modifica', 'Undo change')}
              </Button>
            )}
          </div>
          {s.recovering && (
            <div className="search-recovery-options">
              <p>
                {!alternatives.length
                  ? t(
                      'Questa ricerca contiene gruppi: modifica una data o un filtro nella barra per mantenere le alternative.',
                      'This search contains groups: edit a date or filter in the search box to preserve its alternatives.',
                    )
                  : coverageIncomplete
                    ? t(
                        'Prima controlla la posta non ancora cercata. Oppure modifica un solo criterio:',
                        'First check mail that has not been searched. Or change one criterion:',
                      )
                    : t('Prova una modifica. Puoi annullarla.', 'Try one change. You can undo it.')}
              </p>
              {alternatives.map((alternative, index) => (
                <Button
                  key={`${index}:${alternative.query}`}
                  variant={index === 0 && !coverageIncomplete ? 'primary' : 'secondary'}
                  onClick={() => {
                    recover(alternative.query);
                    // Recovery labels are built with the regional locale, so compare in that language.
                    if (alternative.label === t('Cerca in tutti gli account', 'Search all accounts')) onallaccounts();
                  }}
                >
                  {alternative.label}
                </Button>
              ))}
            </div>
          )}
        </div>
      )}
    </section>
  );
}
