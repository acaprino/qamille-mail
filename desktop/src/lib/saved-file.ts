// What a notice offers after a file was saved (an attachment, an exported message): Open and Show in
// folder. The page never names a path: the save handed back a token, valid only for the files saved in
// this session (src-tauri/src/saved_files.rs), and the commands take that token alone.
import type { NoticeAction } from '../ui/Toaster';
import { call, errorText } from './api';
import { tr } from './i18n';

type SavedFileCommand = 'saved_file_open' | 'saved_file_reveal';

/** The actions of the notice that says a file was saved; none when the save gave no token. A failing
 *  action says why in a notice of its own. The file stays reachable from File Explorer either way. */
export function savedFileActions(
  token: number | null | undefined,
  say: (message: string, error?: boolean) => void,
): NoticeAction[] {
  if (token === null || token === undefined) return [];
  const ask = (command: SavedFileCommand) => () => {
    void call(command, { token }).catch((error) => say(errorText(error), true));
  };
  return [
    { label: tr('Apri', 'Open'), run: ask('saved_file_open') },
    { label: tr('Mostra nella cartella', 'Show in folder'), run: ask('saved_file_reveal') },
  ];
}
