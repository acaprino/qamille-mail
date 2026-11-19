// A number that changes in place (unread counts, totals): unlumen's Animate Digits rolls each digit
// that changes. The current digits are the element's only text (the leaving ones are aria-hidden and drawn
// by CSS), so the text and the accessible name are the plain number.
import { useReducedMotion } from 'motion/react';
import { AnimateDigits } from '../unlumen/animate-digits';
import { cn } from '../cn';

export default function Count({ value, className }: { value: number | string; className?: string }) {
  const still = useReducedMotion();
  const text = String(value);
  if (still) return <span className={className}>{text}</span>;
  return (
    <span className={cn('count-digits', className)}>
      <AnimateDigits value={text} gap={0} enterY={10} enterBlur={6} enterScale={0.85} animationDelay={60} />
    </span>
  );
}
