import { createRoot } from 'react-dom/client';
import { installBrowserGuards } from '../lib/browser-guards';
import { SigridRoot } from '../ui/SigridRoot';
import Reminder from './Reminder';
import './reminder.css';

installBrowserGuards();
createRoot(document.getElementById('reminder')!).render(
  <SigridRoot>
    <Reminder />
  </SigridRoot>,
);
