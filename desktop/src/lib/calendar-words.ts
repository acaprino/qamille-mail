import { tr } from './i18n';

// The calendar of a date field in the interface language (Ark writes English by default); the
// field's placeholder follows the language too. Call it inside a `$derived` so a language change
// is picked up.
export function calendarWords() {
  return {
    trigger: (open: boolean) =>
      open
        ? tr('Chiudi il calendario', 'Close calendar')
        : tr('Apri il calendario', 'Open calendar'),
    content: tr('Calendario', 'Calendar'),
    nextTrigger: (view: string) =>
      view === 'day'
        ? tr('Mese successivo', 'Next month')
        : view === 'month'
          ? tr('Anno successivo', 'Next year')
          : tr('Pagina successiva', 'Next page'),
    prevTrigger: (view: string) =>
      view === 'day'
        ? tr('Mese precedente', 'Previous month')
        : view === 'month'
          ? tr('Anno precedente', 'Previous year')
          : tr('Pagina precedente', 'Previous page'),
    viewTrigger: (view: string) =>
      view === 'day'
        ? tr('Scegli il mese', 'Switch to month view')
        : view === 'month'
          ? tr('Scegli l’anno', 'Switch to year view')
          : tr('Scegli il giorno', 'Switch to day view'),
    clearTrigger: tr('Cancella la data', 'Clear date'),
    placeholder: () => ({ day: tr('gg', 'dd'), month: 'mm', year: tr('aaaa', 'yyyy') }),
  };
}
