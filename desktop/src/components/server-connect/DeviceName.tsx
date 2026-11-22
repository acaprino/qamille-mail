// The name this computer gets in the account's list of devices: one line with the name and «Change», the
// field only when asked for. Almost nobody changes it, so it does not stand among the sign-in fields.
import { useState } from 'react';
import Button from '../../ui/Button';
import Field from '../../ui/Field';
import { tr } from '../../lib/i18n';
import { Input } from '../../ui/Input';

export default function DeviceName({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  const [editing, setEditing] = useState(false);
  if (editing)
    return (
      <Field>
        {tr('Nome di questo dispositivo', 'Name of this device')}
        <Input value={value} autoFocus onChange={(e) => onChange(e.currentTarget.value)} />
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
