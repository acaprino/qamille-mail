import type { ButtonHTMLAttributes, ReactNode } from 'react';
import Kbd from '../ui/Kbd';
import Button from '../ui/Button';
import Count from '../ui/Count';
import Icon from '../ui/Icon';
import { NavItem, NavList } from '../ui/NavList';
import Tooltip from '../ui/Tooltip';
import { desktop, initials } from '../lib/api';
import { activityCenter, toggleActivityPanel } from '../lib/activity-center';
import type { Activity } from '../lib/activity';
import { folderLabel, tr } from '../lib/i18n';
import { useStore } from '../lib/store';
import type { Folder, UndoAction, Workspace } from '../lib/types';
import ActivityCenter from './ActivityCenter';

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
  // Collapsed, a row is only an icon: it keeps its aria-label and gets a hint instead of a title
  // attribute. Expanded, the row says its own name and stays a plain <button>.
  const hint = (text: string) => (collapsed ? text : undefined);
  const row = (text: string, own: ButtonHTMLAttributes<HTMLButtonElement>, content: ReactNode) =>
    collapsed ? (
      <Tooltip text={text}>
        <button {...own}>{content}</button>
      </Tooltip>
    ) : (
      <button {...own}>{content}</button>
    );
  const toggleLabel = collapsed
    ? tr('Espandi la barra laterale', 'Expand sidebar')
    : tr('Comprimi la barra laterale', 'Collapse sidebar');

  return (
    <aside className={collapsed ? 'sidebar collapsed' : 'sidebar'} aria-label={tr('Barra laterale', 'Sidebar')}>
      <div className="sidebar-heading" data-tauri-drag-region={desktop ? '' : undefined}>
        <div className="brand" data-tauri-drag-region={desktop ? '' : undefined}>
          <span className="brand-mark">
            <svg width="11" height="14" viewBox="0 0 21 25" fill="none" aria-hidden="true">
              <path d="M12 1 2 14h8l-1 10 10-14h-8l1-9Z" fill="currentColor" />
            </svg>
          </span>
          <span className="sidebar-text">Qamille</span>
        </div>
        <Button
          variant="icon"
          className="sidebar-toggle"
          onClick={toggleCollapsed}
          aria-label={toggleLabel}
          tip={toggleLabel}
          aria-expanded={!collapsed}
          aria-controls="sidebar-navigation"
        >
          <Icon name={collapsed ? 'sidebar-open' : 'sidebar-close'} size={iconSize} stroke={iconStroke} />
        </Button>
      </div>
      <Button
        variant="primary"
        className="compose-main"
        onClick={compose}
        tip={collapsed ? tr('Nuovo messaggio', 'New message') : undefined}
        shortcut={collapsed ? 'N' : undefined}
        aria-keyshortcuts="N"
        aria-label={collapsed ? tr('Nuovo messaggio', 'New message') : undefined}
      >
        <Icon name="edit" size={iconSize} stroke={iconStroke} />
        <span className="sidebar-text">{tr('Nuovo messaggio', 'New message')}</span>
        <Kbd>N</Kbd>
      </Button>
      <div className="sidebar-body" id="sidebar-navigation">
        <div className="sidebar-label sidebar-text">{tr('Account', 'Accounts')}</div>
        <NavList className="accounts-nav" aria-label={tr('Account', 'Accounts')} group="accounts">
          <NavItem
            hint={hint(tr('Tutti gli account', 'All accounts'))}
            active={accountId === null}
            onClick={() => selectAccount(null)}
            aria-label={tr('Tutti gli account', 'All accounts')}
            aria-pressed={accountId === null}
            icon={
              <span className="account-avatar all-accounts">
                <Icon name="inbox" size={12} stroke={iconStroke} />
              </span>
            }
          >
            <span className="sidebar-text">{tr('Tutti gli account', 'All accounts')}</span>
          </NavItem>
          {workspace.accounts.map((account) => (
            <NavItem
              key={account.id}
              hint={hint(account.name)}
              active={accountId === account.id}
              onClick={() => selectAccount(account.id)}
              aria-label={collapsed ? tr(`Passa a ${account.name}`, `Switch to ${account.name}`) : undefined}
              title={collapsed ? undefined : `${account.name} · ${account.email}`}
              aria-pressed={accountId === account.id}
              icon={
                <span
                  className="account-avatar"
                  style={{ '--account-color': `rgb(${account.color.slice(0, 3).join(' ')})` } as React.CSSProperties}
                >
                  {initials(account.name)}
                </span>
              }
            >
              <span className="account-details sidebar-text">
                <strong>{account.name}</strong>
                <small>{account.email}</small>
              </span>
            </NavItem>
          ))}
        </NavList>
        <NavList aria-label={tr('Cartelle della posta', 'Mail folders')} group="views">
          {primary.map(([id, icon]) => (
            <NavItem
              key={id}
              hint={hint(`${folderLabel(id)}${workspace.counts[id] ? ` · ${workspace.counts[id]}` : ''}`)}
              active={view === id}
              onClick={() => navigate(id)}
              aria-label={collapsed ? folderLabel(id) : undefined}
              icon={<Icon name={icon} size={iconSize} stroke={iconStroke} />}
              end={!!workspace.counts[id] && <Count value={workspace.counts[id]} className="nav-count" />}
            >
              <span className="sidebar-text">{folderLabel(id)}</span>
            </NavItem>
          ))}
        </NavList>
        <div className="sidebar-label sidebar-text">
          {tr('Etichette', 'Labels')} <span>{workspace.labels.length}</span>
        </div>
        <NavList aria-label={tr('Etichette', 'Labels')} className="labels-nav" group="views">
          {workspace.labels.map((label, i) => (
            <NavItem
              key={label}
              hint={hint(label)}
              active={view === `label:${label}`}
              onClick={() => navigate(`label:${label}`)}
              aria-label={label}
              icon={<span className="label-dot" style={{ '--dot': `var(--label-${(i % 5) + 1})` } as React.CSSProperties} />}
            >
              <span className="sidebar-text">{label}</span>
            </NavItem>
          ))}
        </NavList>
      </div>
      <div className="sidebar-bottom">
        <NavList className="tools-nav" aria-label={tr('Spazio di lavoro', 'Workspace')} group="views">
          {tools.map(([id, icon]) => (
            <NavItem
              key={id}
              hint={hint(folderLabel(id))}
              active={view === id}
              onClick={() => navigate(id)}
              aria-label={collapsed ? folderLabel(id) : undefined}
              icon={<Icon name={icon} size={iconSize} stroke={iconStroke} />}
              end={id === 'Assistant' ? <span className="ai-badge">AI</span> : undefined}
            >
              <span className="sidebar-text">{folderLabel(id)}</span>
            </NavItem>
          ))}
        </NavList>
        <ActivityCenter
          activities={activities}
          undoActions={undoActions}
          accounts={workspace.accounts}
          openActivity={openActivity}
          showAll={showAllActivity}
        />
        <div className="profile-row">
          {row(
            folderLabel('Settings'),
            {
              className: view === 'Settings' ? 'active profile' : 'profile',
              onClick: () => navigate('Settings'),
              'aria-label': collapsed ? folderLabel('Settings') : undefined,
              'aria-current': view === 'Settings' ? 'page' : undefined,
            },
            <>
              <span className="avatar user-avatar">{initials(workspace.settings.user_name)}</span>
              <span className="sidebar-text">
                <strong>{workspace.settings.user_name}</strong>
                <small>{tr('Spazio personale', 'Personal workspace')}</small>
              </span>
              {/* The gear is decorative: the name of the button has to say where it leads. */}
              <span className="visually-hidden">{folderLabel('Settings')}</span>
              <Icon name="settings" size={iconSize} stroke={iconStroke} />
            </>,
          )}
          {/* The history button is a sibling of the profile button, not inside it: buttons do not nest.
              A dot on it means an error or a failed action the user has not seen yet. */}
          {row(
            tr('Cronologia delle attività', 'Activity history'),
            {
              className: `activity-toggle${center.open ? ' on' : ''}${center.unseen > 0 ? ' has-unseen' : ''}`,
              onClick: toggleActivityPanel,
              'aria-label':
                center.unseen > 0
                  ? tr('Cronologia delle attività: ci sono errori da vedere', 'Activity history: there are errors to see')
                  : tr('Cronologia delle attività', 'Activity history'),
              'aria-expanded': center.open,
              'aria-controls': 'activity-panel',
            },
            <Icon name="history" size={iconSize} stroke={iconStroke} />,
          )}
        </div>
      </div>
    </aside>
  );
}
