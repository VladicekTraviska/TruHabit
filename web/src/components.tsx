import { t } from './i18n';
import { Children, useState, type ReactNode } from 'react';
import { Check, Eye, EyeSlash } from '@phosphor-icons/react';
export function Brand() {
  return (
    <a className="brand" href="/" aria-label={t("TruHabit, úvod")} translate="no">
      <span className="brand-mark"><Check size={23} weight="bold" aria-hidden="true" /></span>tru<span>habit</span>
      <span className="brand-period">.</span>
    </a>
  );
}
export function Message({ children, error = false }: { children: ReactNode; error?: boolean }) {
  return (
    <div className={`alert ${error ? 'error' : 'notice'}`} role={error ? 'alert' : 'status'}>
      {Children.map(children, child => typeof child === 'string' ? t(child) : child)}
    </div>
  );
}
export function Field({
  label,
  children,
  hint,
}: {
  label: string;
  children: ReactNode;
  hint?: string;
}) {
  return (
    <label className="product-field">
      <span>{label}</span>
      {children}
      {hint && <small>{hint}</small>}
    </label>
  );
}
export function PasswordField({
  label,
  name = 'password',
  newPassword = false,
}: {
  label: string;
  name?: string;
  newPassword?: boolean;
}) {
  const [visible, setVisible] = useState(false);
  return (
    <Field
      label={label}
      hint={newPassword ? t("Alespoň 15 znaků. Můžete použít celou větu.") : undefined}
    >
      <span className="password-control"><input
        name={name}
        aria-label={label}
        type={visible ? 'text' : 'password'}
        autoComplete={newPassword ? 'new-password' : 'current-password'}
        minLength={newPassword ? 15 : 1}
        maxLength={128}
        required
      /><button type="button" className="password-toggle" onClick={() => setVisible(v => !v)} aria-label={visible ? t('Skrýt heslo') : t('Zobrazit heslo')} aria-pressed={visible}>{visible ? <EyeSlash size={20} aria-hidden="true" /> : <Eye size={20} aria-hidden="true" />}</button></span>
    </Field>
  );
}
