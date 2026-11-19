import { createRoot } from 'react-dom/client';
import { installBrowserGuards } from '../lib/browser-guards';
import Reminder from './Reminder';
import './reminder.css';

installBrowserGuards();
createRoot(document.getElementById('reminder')!).render(<Reminder />);
