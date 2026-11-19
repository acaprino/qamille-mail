// Decides which calendar loads may replace what is on screen. A live read takes seconds (Google);
// the 2-second refresh is quick and only re-reads the local calendar, reusing the Google events it
// found on screen when it began. Rules:
//  - a load older than the newest live load is stale (the user changed week or hid a calendar);
//  - a refresh is stale when a live result landed after it began (it carries the older Google data),
//    or when a newer refresh already landed.
// Every other refresh is fine, so a slow live read is never thrown away because refreshes started
// (and finished) while it was running.
export type LoadTicket = { id: number; refresh: boolean; liveSeen: number };

export class LoadGate {
  private next = 0;
  private landed = 0;
  private newestLive = 0;
  private liveLanded = 0;

  begin(refresh: boolean): LoadTicket {
    const id = ++this.next;
    if (!refresh) this.newestLive = id;
    return { id, refresh, liveSeen: this.liveLanded };
  }

  /** True when `ticket`'s result may replace the screen; records it as landed. */
  accept(ticket: LoadTicket): boolean {
    if (ticket.id < this.newestLive) return false;
    if (ticket.refresh && (ticket.id < this.landed || ticket.liveSeen !== this.liveLanded)) return false;
    this.landed = Math.max(this.landed, ticket.id);
    if (!ticket.refresh) this.liveLanded++;
    return true;
  }
}
