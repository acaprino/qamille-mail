// A single-value slider. Left and Down lower it and Right and Up raise it by `step` (Up and Down are
// mapped to Right and Left, as the ARIA pattern has it), PageUp and PageDown move by ten steps, Home
// and End go to `min` and `max`. `onChange` is told each value. The look is in ds/styles/ark/slider.css.
import { Slider as ArkSlider } from '@ark-ui/react/slider';
import type { KeyboardEvent } from 'react';

export type SliderProps = {
  value: number;
  onChange?: (value: number) => void;
  min: number;
  max: number;
  step?: number;
  'aria-label': string;
};

// Up and Down act as Right and Left: the same key as Ark knows, sent again from the thumb.
function vertical(event: KeyboardEvent<HTMLDivElement>) {
  const key = event.key === 'ArrowUp' ? 'ArrowRight' : event.key === 'ArrowDown' ? 'ArrowLeft' : undefined;
  if (!key) return;
  event.preventDefault();
  event.stopPropagation();
  event.currentTarget.dispatchEvent(
    new KeyboardEvent('keydown', { key, shiftKey: event.shiftKey, bubbles: true, cancelable: true }),
  );
}

export default function Slider({ value, onChange, min, max, step = 1, 'aria-label': ariaLabel }: SliderProps) {
  return (
    <ArkSlider.Root
      value={[value]}
      min={min}
      max={max}
      step={step}
      aria-label={[ariaLabel]}
      onValueChange={(details) => onChange?.(details.value[0])}
    >
      <ArkSlider.Control>
        <ArkSlider.Track>
          <ArkSlider.Range />
        </ArkSlider.Track>
        <ArkSlider.Thumb index={0} onKeyDownCapture={vertical}>
          <ArkSlider.HiddenInput />
        </ArkSlider.Thumb>
      </ArkSlider.Control>
    </ArkSlider.Root>
  );
}
