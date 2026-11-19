// The app's line icons: 24x24 paths drawn with the current colour. Unknown names draw the mail icon.
export const iconPaths: Record<string, string> = {
    threads: 'M4 3h16v12H9l-5 4V3ZM8 19h8l4 3v-4',
    'sidebar-close': 'M3 3h18v18H3V3ZM9 3v18m7-13-4 4 4 4',
    'sidebar-open': 'M3 3h18v18H3V3ZM9 3v18m4-13 4 4-4 4',
    print: 'M6 9V3h12v6M6 18H3V9h18v9h-3M6 14h12v7H6v-7M17 11h.01',
    download: 'M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5',
    upload: 'M12 15V3M7 8l5-5 5 5M4 16v5h16v-5',
    underline: 'M6 3v7a6 6 0 0 0 12 0V3M4 21h16',
    'list-ordered': 'M10 6h11M10 12h11M10 18h11M4 6h1v4M4 10h2M6 18H4c0-1 2-2 2-3s-1-1.5-2-1',
    quote: 'M17 6H3M21 12H8M21 18H8M3 12v6',
    undo: 'M9 14 4 9l5-5M4 9h10.5a5.5 5.5 0 0 1 0 11H11',
    redo: 'm15 14 5-5-5-5M20 9H9.5a5.5 5.5 0 0 0 0 11H13',
    inbox: 'M4 4h16l2 11v5H2v-5L4 4Zm-2 11h6l2 3h4l2-3h6',
    mail: 'M3 5h18v14H3V5Zm0 1 9 7 9-7',
    check: 'm5 12 4 4L19 6',
    minus: 'M5 12h14',
    task: 'M9 5h11M9 12h11M9 19h11M3 5l1 1 2-3M3 12l1 1 2-3M3 19l1 1 2-3',
    clock: 'M12 8v5l3 2M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0',
    history: 'M3 12a9 9 0 1 0 3-6.7M3 4v5h5M12 7v5l3 2',
    edit: 'm15 4 5 5M4 20l5-1L21 7l-5-5L4 14v6ZM12 20h9',
    send: 'm22 2-7 20-4-9-9-4L22 2Zm0 0L11 13',
    archive: 'M3 3h18v5H3V3Zm2 5v13h14V8M9 12h6',
    trash: 'M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7',
    calendar: 'M3 5h18v16H3V5ZM7 2v6M17 2v6M3 11h18M7 15h2M13 15h2',
    contacts: 'M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8ZM22 21v-2a4 4 0 0 0-3-3.9M16 3.1a4 4 0 0 1 0 7.8',
    mic: 'M9 4a3 3 0 0 1 6 0v8a3 3 0 0 1-6 0V4ZM5 10v2a7 7 0 0 0 14 0v-2M12 19v3M8 22h8',
    spark: 'm12 2 2.5 7.5L22 12l-7.5 2.5L12 22l-2.5-7.5L2 12l7.5-2.5L12 2Z',
    settings:
      'M10.23 4.61L10.44 2.12L13.56 2.12L13.77 4.61L15.97 5.52L17.88 3.91L20.09 6.12L18.48 8.03L19.39 10.23L21.88 10.44L21.88 13.56L19.39 13.77L18.48 15.97L20.09 17.88L17.88 20.09L15.97 18.48L13.77 19.39L13.56 21.88L10.44 21.88L10.23 19.39L8.03 18.48L6.12 20.09L3.91 17.88L5.52 15.97L4.61 13.77L2.12 13.56L2.12 10.44L4.61 10.23L5.52 8.03L3.91 6.12L6.12 3.91L8.03 5.52ZM15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0',
    search: 'M17 10a7 7 0 1 1-14 0 7 7 0 0 1 14 0Zm-2 5 6 6',
    plus: 'M12 5v14M5 12h14',
    chevron: 'm9 5 7 7-7 7',
    down: 'm6 9 6 6 6-6',
    pin: 'm8 3 8 0-1 6 4 4v2H5v-2l4-4-1-6ZM12 15v7',
    clip: 'm8 12 6-6a3 3 0 0 1 4 4l-9 9a5 5 0 0 1-7-7L13 1M6 14l9-9',
    refresh: 'M20 7a9 9 0 0 0-15-2L2 8m0-6v6h6M4 17a9 9 0 0 0 15 2l3-3m0 6v-6h-6',
    reply: 'm9 4-7 7 7 7v-5h6c4 0 6 3 7 7V10c0-3-3-5-6-5H9V4Z',
    // The reply arrow with a second head behind it: Reply all must not look like Reply.
    replyAll: 'm12 5-6 6 6 6v-4h4c3 0 5 2 6 6v-9c0-3-2-5-5-5h-5ZM7 5l-6 6 6 6',
    arrow: 'M4 12h16m-6-6 6 6-6 6',
    close: 'm6 6 12 12M6 18 18 6',
    minimize: 'M5 17h14',
    'window-maximize': 'M5 5h14v14H5V5Z',
    'window-restore': 'M8 8h11v11H8V8ZM5 16V5h11',
    moon: 'M21 13A9 9 0 0 1 11 3a9 9 0 1 0 10 10Z',
    sun: 'M16 12a4 4 0 1 1-8 0 4 4 0 0 1 8 0ZM12 2v2M12 20v2M2 12h2M20 12h2M5 5l1 1M18 18l1 1M5 19l1-1M18 6l1-1',
    // Rings of radius 0.6, so each dot is about 3px wide: a zero-length stroke looked disabled.
    more: 'M5 11.4a.6.6 0 1 0 0 1.2.6.6 0 0 0 0-1.2ZM12 11.4a.6.6 0 1 0 0 1.2.6.6 0 0 0 0-1.2ZM19 11.4a.6.6 0 1 0 0 1.2.6.6 0 0 0 0-1.2Z',
    tag: 'M2 3h9l11 11-8 8L2 10V3ZM7 7h.01',
    link: 'm10 14 4-4M8 16l-2 2a4 4 0 0 1-5-5l5-5a4 4 0 0 1 5 0m2 8a4 4 0 0 0 5 0l5-5a4 4 0 0 0-5-5l-2 2',
    bold: 'M6 3h7a5 5 0 0 1 0 9H6V3Zm0 9h8a5 5 0 0 1 0 9H6v-9',
    italic: 'M10 3h10M4 21h10M15 3 9 21',
    list: 'M9 5h12M9 12h12M9 19h12M3 5h.01M3 12h.01M3 19h.01',
    info: 'M12 11v6M12 7h.01M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0',
    logout: 'M9 3H3v18h6M9 12h13m-5-5 5 5-5 5',
    copy: 'M11 9h7a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2h-7a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2ZM5 15V6a2 2 0 0 1 2-2h8',
    pause: 'M9 5v14M15 5v14',
    play: 'M7 4.5v15l12-7.5-12-7.5Z',
    stop: 'M8 6h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2Z',
    video: 'M5 6h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2Zm10 4 6-3v10l-6-3',
    alert: 'M12 4 3 20h18L12 4ZM12 10v4M12 17v.5',
  };

export type IconProps = { name: string; size?: number; stroke?: number };

export default function Icon({ name, size = 20, stroke = 1.7 }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={stroke}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={iconPaths[name] || iconPaths.mail} />
    </svg>
  );
}
