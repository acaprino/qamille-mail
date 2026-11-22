import type { ReactNode } from 'react';
import { DEFAULT_WORDS, MotionProvider, WordsProvider } from '@acaprino/sigrid';

/**
 * The root every Qamille window renders under: the words Sigrid's primitives say (its own until Qamille gives them
 * in Italian and English) and Sigrid's motion, which still honours the system's reduced-motion setting.
 */
export function SigridRoot({ children }: { children: ReactNode }) {
  return (
    <WordsProvider words={DEFAULT_WORDS}>
      <MotionProvider animated>{children}</MotionProvider>
    </WordsProvider>
  );
}
