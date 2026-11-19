// The quiet inbox groups (Notifications, Newsletters, Seen) start closed (spec 2026-11-07 §4.2).
// Tests about something else than the groups need every row on screen, so they open what is
// closed. The loop re-reads the headings each time: opening one moves the others.
export async function openAllGroups(page) {
  await page.locator('.mail-row').first().waitFor();
  const closed = page.locator('.group-heading[aria-expanded="false"]');
  while (await closed.count()) await closed.first().click();
}
