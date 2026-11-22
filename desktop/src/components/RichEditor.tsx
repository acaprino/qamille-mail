import { Fragment, useEffect, useImperativeHandle, useRef, useState, type KeyboardEvent, type ReactNode, type Ref } from 'react';
import { Editor } from '@tiptap/core';
import StarterKit from '@tiptap/starter-kit';
import DOMPurify from 'dompurify';
import Icon from '../ui/Icon';
import Tooltip from '../ui/Tooltip';
import { tr } from '../lib/i18n';

export type RichEditorHandle = {
  /** Puts the caret at the end of the text. */
  focus: () => void;
};

export type RichEditorProps = {
  html: string;
  onchange: (html: string) => void;
  disabled?: boolean;
  ref?: Ref<RichEditorHandle>;
};

type Tool = { group: number; label: string; icon: ReactNode; run: () => void; pressed?: boolean; can?: boolean };

export default function RichEditor({ html, onchange, disabled = false, ref }: RichEditorProps) {
  const element = useRef<HTMLDivElement>(null);
  const [editor, setEditor] = useState<Editor | null>(null);
  // The same instance as `editor`, there from the moment it is created (the state follows a render later).
  const instance = useRef<Editor | null>(null);
  useImperativeHandle(
    ref,
    () => ({ focus: () => void instance.current?.commands.focus('end', { scrollIntoView: false }) }),
    [],
  );
  // The one button of the toolbar that is in the Tab order: the one that last had the focus.
  const [lastStop, setLastStop] = useState(0);
  const [, setRevision] = useState(0);
  // The editor is created once, with the html of the first render; later changes come from it.
  const initialHtml = useRef(html);
  const onchangeRef = useRef(onchange);
  onchangeRef.current = onchange;

  useEffect(() => {
    const created = new Editor({
      element: element.current!,
      extensions: [
        StarterKit.configure({
          link: { openOnClick: false, protocols: ['http', 'https', 'mailto'] },
        }),
      ],
      content: DOMPurify.sanitize(initialHtml.current),
      editorProps: {
        attributes: {
          class: 'rich-content',
          role: 'textbox',
          'aria-label': tr('Corpo del messaggio', 'Message body'),
          'aria-multiline': 'true',
        },
      },
      onUpdate: ({ editor }) => {
        setRevision((r) => r + 1);
        onchangeRef.current(editor.getHTML());
      },
      onSelectionUpdate: () => setRevision((r) => r + 1),
    });
    setEditor(created);
    instance.current = created;
    return () => {
      instance.current = null;
      created.destroy();
    };
  }, []);

  useEffect(() => {
    // Tiptap's setEditable emits an update by default. Updating the draft from
    // that event would make this effect depend on its own writes.
    editor?.setEditable(!disabled, false);
  }, [editor, disabled]);

  const label = tr('Corpo del messaggio', 'Message body');
  useEffect(() => {
    editor?.view.dom.setAttribute('aria-label', label);
  }, [editor, label]);

  const active = (name: string) => editor?.isActive(name) ?? false;
  const run = () => editor?.chain().focus();
  // The buttons of the toolbar, in groups. Most show only an icon or a sign, so the name is also the hint;
  // a format that can be on or off says so (`pressed`), undo and redo turn off when there is nothing to do.
  const tools: Tool[] = [
    { group: 0, label: tr('Grassetto', 'Bold'), icon: <Icon name="bold" size={15} />, run: () => run()?.toggleBold().run(), pressed: active('bold') },
    { group: 0, label: tr('Corsivo', 'Italic'), icon: <Icon name="italic" size={15} />, run: () => run()?.toggleItalic().run(), pressed: active('italic') },
    { group: 0, label: tr('Sottolineato', 'Underline'), icon: <Icon name="underline" size={15} />, run: () => run()?.toggleUnderline().run(), pressed: active('underline') },
    { group: 1, label: tr('Elenco puntato', 'Bullet list'), icon: <Icon name="list" size={17} />, run: () => run()?.toggleBulletList().run(), pressed: active('bulletList') },
    { group: 1, label: tr('Elenco numerato', 'Numbered list'), icon: <Icon name="list-ordered" size={17} />, run: () => run()?.toggleOrderedList().run(), pressed: active('orderedList') },
    { group: 1, label: tr('Citazione', 'Quote'), icon: <Icon name="quote" size={17} />, run: () => run()?.toggleBlockquote().run(), pressed: active('blockquote') },
    { group: 2, label: tr('Annulla', 'Undo'), icon: <Icon name="undo" size={16} />, run: () => run()?.undo().run(), can: editor?.can().undo() ?? false },
    { group: 2, label: tr('Ripeti', 'Redo'), icon: <Icon name="redo" size={16} />, run: () => run()?.redo().run(), can: editor?.can().redo() ?? false },
  ];
  const works = tools.map((item) => !disabled && item.can !== false);
  // One Tab stop for the whole toolbar (roving tabindex): the button that last had the focus, or the first
  // that works when that one is off. Tab goes on to the text, the arrow keys move inside the toolbar.
  const stop = works[lastStop] ? lastStop : works.indexOf(true);
  function moveInToolbar(event: KeyboardEvent<HTMLDivElement>) {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
    const at = buttons.indexOf(event.target as HTMLButtonElement);
    if (at < 0) return;
    const to =
      event.key === 'Home' ? 0
      : event.key === 'End' ? buttons.length - 1
      : event.key === 'ArrowRight' ? (at + 1) % buttons.length
      : event.key === 'ArrowLeft' ? (at - 1 + buttons.length) % buttons.length
      : -1;
    if (to < 0) return;
    event.preventDefault();
    buttons[to].focus();
  }

  return (
    <div className="rich-editor">
      <div
        className="format-toolbar"
        role="toolbar"
        aria-label={tr('Formattazione del testo', 'Text formatting')}
        onKeyDown={moveInToolbar}
      >
        {tools.map((item, at) => (
          <Fragment key={item.label}>
            {at > 0 && item.group !== tools[at - 1].group && <span className="toolbar-divider"></span>}
            <Tooltip text={item.label}>
              <button
                type="button"
                className={item.pressed ? 'active' : undefined}
                disabled={!works[at]}
                tabIndex={at === stop ? 0 : -1}
                aria-label={item.label}
                aria-pressed={item.pressed}
                onFocus={() => setLastStop(at)}
                onClick={item.run}
              >
                {item.icon}
              </button>
            </Tooltip>
          </Fragment>
        ))}
      </div>
      <div ref={element}></div>
    </div>
  );
}
