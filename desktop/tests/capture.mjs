import { chromium } from '@playwright/test';
const browser = await chromium.connectOverCDP(process.env.QAMILLE_CDP);
const page = browser.contexts()[0].pages()[0];
await page.getByRole('heading', {name:'Q4 proposal: can you review by Thursday?'}).waitFor();
await page.screenshot({path:'../checks/tauri-preview.png'});
await browser.close();
