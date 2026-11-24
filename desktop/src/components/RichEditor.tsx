import { Fragment, useEffect, useImperativeHandle, useRef, useState, type ReactNode, type Ref } from 'react';
import { Editor } from '@tiptap/core';
import StarterKit from '@tiptap/starter-kit';
import DOMPurify from 'dompurify';
import Icon from '../ui/Icon';
import { Toolbar, ToolbarButton, ToolbarSeparator } from '../ui/Toolbar';
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
  return (
    <div className="rich-editor">
      {/* Sigrid's toolbar: one Tab stop for the whole bar, the arrows move between the tools; 28 px tall in the dense size. */}
      <Toolbar size="sm" className="format-toolbar" aria-label={tr('Formattazione del testo', 'Text formatting')}>
        {tools.map((item, at) => (
          <Fragment key={item.label}>
            {at > 0 && item.group !== tools[at - 1].group && <ToolbarSeparator />}
            <ToolbarButton label={item.label} icon={item.icon} pressed={item.pressed} disabled={!works[at]} onClick={item.run} />
          </Fragment>
        ))}
      </Toolbar>
      <div ref={element}></div>
    </div>
  );
}
