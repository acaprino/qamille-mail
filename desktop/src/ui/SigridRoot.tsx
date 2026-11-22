import type { ReactNode } from 'react';
import { MotionProvider, WordsProvider } from '@acaprino/sigrid';
import { useLanguage } from '../lib/i18n';
import { qamilleWords } from './words';

/**
 * The root every Qamille window renders under: the words Sigrid's primitives say, in the interface language, and
 * Sigrid's motion, which still honours the system's reduced-motion setting. It subscribes to the language itself:
 * it sits above `App`, whose own subscription does not reach it.
 */
export function SigridRoot({ children }: { children: ReactNode }) {
  const italian = useLanguage();
  return (
    <WordsProvider words={qamilleWords(italian)}>
      <MotionProvider animated>{children}</MotionProvider>
    </WordsProvider>
  );
}
