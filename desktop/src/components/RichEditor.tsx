import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Editor } from '@tiptap/core';
import StarterKit from '@tiptap/starter-kit';
import DOMPurify from 'dompurify';
import Icon from '../ds/primitives/Icon';
import Tooltip from '../ds/primitives/Tooltip';
import { tr } from '../lib/i18n';

export type RichEditorProps = {
  html: string;
  onchange: (html: string) => void;
  disabled?: boolean;
};

export default function RichEditor({ html, onchange, disabled = false }: RichEditorProps) {
  const element = useRef<HTMLDivElement>(null);
  const [editor, setEditor] = useState<Editor | null>(null);
  const [, setRevision] = useState(0);
  // The editor is created once, with the html of the first render; later changes come from it.
  const initialHtml = useRef(html);
  const onchangeRef = useRef(onchange);
  onchangeRef.current = onchange;

  useEffect(() => {
    const instance = new Editor({
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
    setEditor(instance);
    return () => instance.destroy();
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
  // One button of the toolbar. Most show only an icon or a sign, so the name is also the hint; a
  // format that can be on or off says so (`pressed`), undo and redo turn off when there is nothing to do.
  const tool = (label: string, content: ReactNode, onClick: () => void, state: { pressed?: boolean; can?: boolean } = {}) => (
    <Tooltip text={label}>
      <button
        type="button"
        className={state.pressed ? 'active' : undefined}
        disabled={disabled || state.can === false}
        aria-label={label}
        aria-pressed={state.pressed}
        onClick={onClick}
      >
        {content}
      </button>
    </Tooltip>
  );

  return (
    <div className="rich-editor">
      <div className="format-toolbar" role="toolbar" aria-label={tr('Formattazione del testo', 'Text formatting')}>
        {tool(tr('Grassetto', 'Bold'), <Icon name="bold" size={15} />, () => run()?.toggleBold().run(), { pressed: active('bold') })}
        {tool(tr('Corsivo', 'Italic'), <Icon name="italic" size={15} />, () => run()?.toggleItalic().run(), { pressed: active('italic') })}
        {tool(tr('Sottolineato', 'Underline'), <u>U</u>, () => run()?.toggleUnderline().run(), { pressed: active('underline') })}
        <span className="toolbar-divider"></span>
        {tool(tr('Elenco puntato', 'Bullet list'), <Icon name="list" size={17} />, () => run()?.toggleBulletList().run(), {
          pressed: active('bulletList'),
        })}
        {tool(tr('Elenco numerato', 'Numbered list'), '1.', () => run()?.toggleOrderedList().run(), { pressed: active('orderedList') })}
        {tool(tr('Citazione', 'Quote'), '“', () => run()?.toggleBlockquote().run(), { pressed: active('blockquote') })}
        <span className="toolbar-divider"></span>
        {tool(tr('Annulla', 'Undo'), '↶', () => run()?.undo().run(), { can: editor?.can().undo() ?? false })}
        {tool(tr('Ripeti', 'Redo'), '↷', () => run()?.redo().run(), { can: editor?.can().redo() ?? false })}
      </div>
      <div ref={element}></div>
    </div>
  );
}
