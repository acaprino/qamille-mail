// The room a bar at the foot of the window takes, written once for Sigrid's notices (`--bottom-bar-room`, which its column
// lifts itself by). Each bar reports its height under its own name and the variable is the sum: the strip of open drafts
// today, and whatever else sits at the foot. A bar that goes away reports 0.
const room = new Map<string, number>();

export function reportBottomRoom(source: string, pixels: number): void {
  if (pixels > 0) room.set(source, Math.ceil(pixels));
  else room.delete(source);
  const total = [...room.values()].reduce((sum, height) => sum + height, 0);
  document.documentElement.style.setProperty('--bottom-bar-room', `${total}px`);
}
