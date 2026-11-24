// The sidebar, drawn by Sigrid's Sidebar: the column when it is wide, the rail of icons when it is shrunk (its own button at the
// foot shrinks and widens it). Above the groups, the app's name and the account in use (a Select: «All accounts» or one);
// then the groups of rows: write, the folders, the labels, the places of the workspace with the history and the settings.
// At the very foot, the user (an avatar and the name, Sigrid's SidebarRow) with the user menu and the button that shrinks the column at its end, and the activity center above it; on the rail, Sigrid's own button widens it again.
// A row is a NavItem with a plain name, so the rail shows it as an icon with its name
// as a hint.
import type { CSSProperties } from 'react';
import Avatar from '../ui/Avatar';
import Button from '../ui/Button';
import Count from '../ui/Count';
import Icon from '../ui/Icon';
import Kbd from '../ui/Kbd';
import Menu from '../ui/Menu';
import { NavItem } from '../ui/NavList';
import Select from '../ui/Select';
import { Sidebar as ShellSidebar, SidebarGroup, SidebarRow } from '../ui/Sidebar';
import { desktop } from '../lib/api';
import { activityCenter, toggleActivityPanel } from '../lib/activity-center';
import type { Activity } from '../lib/activity';
import { folderLabel, tr } from '../lib/i18n';
import { useStore } from '../lib/store';
import type { Folder, UndoAction, Workspace } from '../lib/types';
import ActivityCenter from './ActivityCenter';

/** An account's colour, [r, g, b, …] in 0..255, as #rrggbb. */
const hexOf = (rgb: number[]) => `#${rgb.slice(0, 3).map((n) => Math.max(0, Math.min(255, Math.round(n))).toString(16).padStart(2, '0')).join('')}`;

export type SidebarProps = {
  workspace: Workspace;
  view: Folder;
  accountId: number | null;
  navigate: (v: Folder) => void;
  selectAccount: (id: number | null) => void;
  compose: () => void;
  collapsed: boolean;
  toggleCollapsed: () => void;
  /** The live work of the activity center (sync, indexing, model, contacts). */
  activities: Activity[];
  undoActions: UndoAction[];
  openActivity: (activity: Activity) => void;
  showAllActivity: () => void;
};

const primary: [Folder, string][] = [
  ['Inbox', 'inbox'],
  ['Tasks', 'task'],
  ['Snoozed', 'clock'],
  ['Drafts', 'edit'],
  ['Sent', 'send'],
  ['Archive', 'archive'],
  ['Trash', 'trash'],
];
const tools: [Folder, string][] = [
  ['Calendar', 'calendar'],
  ['Contacts', 'contacts'],
  ['Meetings', 'mic'],
  ['Assistant', 'spark'],
];
const iconSize = 16;
const iconStroke = 1.8;

/** A coloured dot: the colour of a label or of an account. */
const dot = (color: string) => <span className="label-dot" style={{ '--dot': color } as CSSProperties} />;

export default function Sidebar({
  workspace,
  view,
  accountId,
  navigate,
  selectAccount,
  compose,
  collapsed,
  toggleCollapsed,
  activities,
  undoActions,
  openActivity,
  showAllActivity,
}: SidebarProps) {
  const center = useStore(activityCenter);
  const userName = workspace.settings.user_name || tr('Utente', 'User');
  const icon = (name: string) => <Icon name={name} size={iconSize} stroke={iconStroke} />;
  const header = (
    <div className="flex flex-col gap-3">
      <div className="brand" data-tauri-drag-region={desktop ? '' : undefined}>
        <span className="brand-mark">
          <svg width="11" height="14" viewBox="0 0 21 25" fill="none" aria-hidden="true">
            <path d="M12 1 2 14h8l-1 10 10-14h-8l1-9Z" fill="currentColor" />
          </svg>
        </span>
        <span>Qamille</span>
      </div>
      <Select
        aria-label={tr('Account', 'Account')}
        value={accountId === null ? 'all' : String(accountId)}
        onChange={(value) => selectAccount(value === 'all' ? null : Number(value))}
        items={[
          { value: 'all', label: tr('Tutti gli account', 'All accounts') },
          ...workspace.accounts.map((account) => ({ value: String(account.id), label: account.name, mark: dot(hexOf(account.color)) })),
        ]}
      />
    </div>
  );
  return (
    <ShellSidebar
      aria-label={tr('Barra laterale', 'Sidebar')}
      className="border-e border-line"
      collapsed={collapsed}
      onCollapsedChange={collapsed ? () => toggleCollapsed() : undefined}
      header={header}
      footer={
        <>
          <ActivityCenter
            activities={activities}
            undoActions={undoActions}
            accounts={workspace.accounts}
            openActivity={openActivity}
            showAll={showAllActivity}
          />
          <SidebarRow
            leading={<Avatar name={userName} size="xs" decorative />}
            end={
              <>
                <Menu
                  label={tr('Menu utente', 'User menu')}
                  tip={tr('Menu utente', 'User menu')}
                  trigger={
                    <Button variant="icon" aria-label={tr('Menu utente', 'User menu')}>
                      <Icon name="more" size={16} />
                    </Button>
                  }
                  items={[
                    { value: 'settings', label: folderLabel('Settings'), icon: 'settings', onSelect: () => navigate('Settings') },
                    { value: 'history', label: tr('Cronologia delle attività', 'Activity history'), icon: 'history', onSelect: toggleActivityPanel },
                  ]}
                />
                <Button
                  variant="icon"
                  aria-label={tr('Comprimi la barra laterale', 'Collapse the sidebar')}
                  tip={tr('Comprimi la barra laterale', 'Collapse the sidebar')}
                  aria-expanded={!collapsed}
                  onClick={() => toggleCollapsed()}
                >
                  <Icon name="sidebar-close" size={16} />
                </Button>
              </>
            }
          >
            {userName}
          </SidebarRow>
        </>
      }
    >
      <SidebarGroup>
        <NavItem className="compose-main" icon={icon('edit')} end={<Kbd>N</Kbd>} aria-keyshortcuts="N" onClick={compose}>
          {tr('Nuovo messaggio', 'New message')}
        </NavItem>
      </SidebarGroup>
      <SidebarGroup label={tr('Cartelle', 'Folders')}>
        {primary.map(([id, name]) => (
          <NavItem
            key={id}
            active={view === id}
            onClick={() => navigate(id)}
            icon={icon(name)}
            end={workspace.counts[id] ? <Count value={workspace.counts[id]} /> : undefined}
          >
            {folderLabel(id)}
          </NavItem>
        ))}
      </SidebarGroup>
      <SidebarGroup label={tr('Etichette', 'Labels')} collapsible defaultOpen={false}>
        {workspace.labels.map((label, i) => (
          <NavItem key={label} active={view === `label:${label}`} onClick={() => navigate(`label:${label}`)} icon={dot(`var(--label-${(i % 5) + 1})`)}>
            {label}
          </NavItem>
        ))}
      </SidebarGroup>
      <SidebarGroup label={tr('Spazio di lavoro', 'Workspace')}>
        {tools.map(([id, name]) => (
          <NavItem key={id} active={view === id} onClick={() => navigate(id)} icon={icon(name)} end={id === 'Assistant' ? <Count value="AI" /> : undefined}>
            {folderLabel(id)}
          </NavItem>
        ))}
        {/* A count on the history means an error or a failed action the user has not seen yet. */}
        <NavItem
          className="activity-toggle"
          icon={icon('history')}
          onClick={toggleActivityPanel}
          aria-expanded={center.open}
          aria-controls="activity-panel"
          aria-label={
            center.unseen > 0
              ? tr('Cronologia delle attività: ci sono errori da vedere', 'Activity history: there are errors to see')
              : tr('Cronologia delle attività', 'Activity history')
          }
          end={center.unseen > 0 ? <Count value={center.unseen} /> : undefined}
        >
          {tr('Cronologia delle attività', 'Activity history')}
        </NavItem>
        <NavItem active={view === 'Settings'} icon={icon('settings')} onClick={() => navigate('Settings')}>
          {folderLabel('Settings')}
        </NavItem>
      </SidebarGroup>
    </ShellSidebar>
  );
}
