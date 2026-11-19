// The name this computer gets in the account's list of devices: one line with the name and «Change», the
// field only when asked for. Almost nobody changes it, so it does not stand among the sign-in fields.
import { useState } from 'react';
import Button from '../../ds/primitives/Button';
import Field from '../../ds/primitives/Field';
import { tr } from '../../lib/i18n';

export default function DeviceName({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  const [editing, setEditing] = useState(false);
  if (editing)
    return (
      <Field>
        {tr('Nome di questo dispositivo', 'Name of this device')}
        <input value={value} autoFocus onChange={(e) => onChange(e.currentTarget.value)} />
      </Field>
    );
  return (
    <p className="device-name-line">
      <span>
        {tr('Questo computer:', 'This computer:')} <strong>{value}</strong>
      </span>
      <Button variant="text" type="button" onClick={() => setEditing(true)}>
        {tr('Cambia', 'Change')}
      </Button>
    </p>
  );
}
