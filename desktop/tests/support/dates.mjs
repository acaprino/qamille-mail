// Sigrid's date field is a button that shows the date and opens a calendar: a date is chosen there, not typed. `pickDate` opens the
// field, walks the calendar to the month of `iso` (`YYYY-MM-DD`) and clicks that day; the card closes and the field shows the date.
// `dateText` is what the button reads, as written in the field's locale («December 24, 2026»).
const PART = (name) => `[data-scope="date-picker"][data-part="${name}"]`;

export async function pickDate(page, field, iso) {
  await field.click();
  const cell = page.locator(`${PART('table-cell-trigger')}[data-value="${iso}"]`).first();
  await page.locator(PART('content')).first().waitFor();
  const next = page.locator(PART('next-trigger')).first();
  const previous = page.locator(PART('prev-trigger')).first();
  for (let step = 0; step < 36 && !(await cell.count()); step += 1) await next.click();
  for (let step = 0; step < 72 && !(await cell.count()); step += 1) await previous.click();
  await cell.click();
}

export const dateText = async (field) => ((await field.textContent()) ?? '').replace(/\s+/g, ' ').trim();
