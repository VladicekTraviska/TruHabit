import { t } from './i18n';
import { useState } from 'react';
import type { FormEvent } from 'react';
import { api } from './api';
import { Field, PasswordField, Message } from './components';
import { ArrowRight, ShieldCheck } from '@phosphor-icons/react';
import './sections-ui.css';

export interface ActionLink {
  purpose: 'verify_email' | 'reset_password';
  token: string;
}
export function readActionLink(): ActionLink | null {
  const params = new URLSearchParams(location.hash.slice(1));
  for (const purpose of ['verify_email', 'reset_password'] as const) {
    const token = params.get(purpose);
    if (token && /^[a-f0-9]{64}$/.test(token)) {
      history.replaceState(null, '', location.pathname);
      return { purpose, token };
    }
  }
  return null;
}
export function AuthPanel({
  onLogin,
  link,
  onDismissLink,
  emailAvailable,
}: {
  onLogin: () => Promise<void>;
  link: ActionLink | null;
  onDismissLink: () => void;
  emailAvailable: boolean;
}) {
  const [mode, setMode] = useState<'login' | 'register' | 'forgot'>('login');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const [email, setEmail] = useState('');
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    setError('');
    setMessage('');
    const form = new FormData(event.currentTarget);
    try {
      if (link) {
        if (link.purpose === 'verify_email')
          await api('/api/auth/verify-email', { method: 'POST', body: { token: link.token } });
        else
          await api('/api/auth/reset-password', {
            method: 'POST',
            body: { token: link.token, new_password: String(form.get('password')) },
          });
        setMessage(
          link.purpose === 'verify_email'
            ? t("E-mail je ověřený. Můžete se přihlásit.")
            : t("Heslo bylo změněné. Přihlaste se novým heslem."),
        );
        onDismissLink();
        setMode('login');
      } else if (mode === 'register') {
        const response = await api<{ message: string }>('/api/auth/register', {
          method: 'POST',
          body: {
            email,
            password: String(form.get('password')),
            display_name: String(form.get('display_name')),
          },
        });
        setMessage(response.message);
        setMode('login');
      } else if (mode === 'forgot') {
        const response = await api<{ message: string }>('/api/auth/forgot-password', {
          method: 'POST',
          body: { email },
        });
        setMessage(response.message);
      } else {
        await api('/api/auth/login', {
          method: 'POST',
          body: { email, password: String(form.get('password')) },
        });
        await onLogin();
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : t("Operaci se nepodařilo dokončit."));
    } finally {
      setBusy(false);
    }
  }
  const title = link
    ? link.purpose === 'verify_email'
      ? t("Potvrďte svůj e-mail")
      : t("Nové heslo")
    : mode === 'register'
      ? t("Váš další krok začíná tady.")
      : mode === 'forgot'
        ? t("Obnovit přístup")
        : t("Vítejte zpátky.");
  return (
    <div className="auth-layout sections-auth">
      <section className="auth-story">
        <p className="eyebrow">{t("PROSTOR PRO VAŠE CÍLE")}</p>
        <h1>{t("Držte slovo.")} <br />
          <span>{t("Hlavně sobě.")}</span>
        </h1>
        <p>{t("Naplánujte si běh, vyberte dosažitelnou vzdálenost a dejte svému cíli konkrétní termín.")} </p>
        <div className="auth-track" aria-hidden="true">
          <svg viewBox="0 0 520 240" fill="none">
            <path className="track-lane" d="M-35 190h282c76 0 76-140 0-140H80c-70 0-70 120 0 120h440" />
            <path className="track-lane" d="M-35 208h282c100 0 100-176 0-176H80c-94 0-94 156 0 156h440" />
            <path className="track-lane" d="M-35 226h282c124 0 124-212 0-212H80c-118 0-118 192 0 192h440" />
            <path className="track-progress" d="M-35 190h282c76 0 76-140 0-140H170" />
            <circle cx="170" cy="50" r="12" fill="#164b3b" stroke="#edf4ef" strokeWidth="5" />
          </svg>
          <span className="track-caption">{t('Vaše vlastní tempo')}</span>
        </div>
        <div className="auth-points">
          <div>
            <span>01</span>
            <strong>{t("Vaše vlastní tempo")}</strong>
            <p>{t("Jeden běh. Jeden cíl. Bez zbytečného závodění.")}</p>
          </div>
          <div>
            <span>02</span>
            <strong>{t("Jasná pravidla")}</strong>
            <p>{t("U každého cíle vidíte termín, stav a historii změn.")}</p>
          </div>
          <div>
            <span>03</span>
            <strong>{t("Účet pod vaší kontrolou")}</strong>
            <p>{t("Spravujte své údaje, peněženku i přihlášená zařízení.")}</p>
          </div>
        </div>
        <p className="auth-availability">{t("Prototyp používá pouze testovací prostředky. Běh můžete doložit souborem GPX nebo FIT. Skutečné platby nejsou zapnuté.")} </p>
      </section>
      <section className="panel auth-card" aria-labelledby="auth-title" aria-busy={busy}>
        <p className="eyebrow">{t("TRUHABIT ÚČET")}</p>
        <h2 id="auth-title">{title}</h2>
        {!link && mode !== 'forgot' && <div className="auth-mode-switch" aria-label={t('TRUHABIT ÚČET')}>
          <button type="button" aria-pressed={mode === 'login'} disabled={busy} onClick={() => { setMode('login'); setError(''); setMessage(''); }}>{t('Přihlásit se')}</button>
          <button type="button" aria-pressed={mode === 'register'} disabled={busy} onClick={() => { setMode('register'); setError(''); setMessage(''); }}>{t('Vytvořit účet')}</button>
        </div>}
        {error && <Message error>{error}</Message>}
        {message && <Message>{message}</Message>}
        <form onSubmit={submit} key={`${mode}-${link?.purpose ?? ''}`}>
          <fieldset disabled={busy}>
            {link?.purpose === 'verify_email' ? (
              <p className="body-copy">{t("Potvrzením prokážete, že máte k této e-mailové adrese přístup.")} </p>
            ) : (
              <>
                {!link && mode === 'register' && (
                  <Field label={t("Jak vám máme říkat?")}>
                    <input name="display_name" autoComplete="nickname" maxLength={80} required />
                  </Field>
                )}
                {!link && (
                  <Field label={t("E-mail")}>
                    <input
                      type="email"
                      name="email"
                      autoComplete="email"
                      spellCheck={false}
                      placeholder="you@example.com"
                      value={email}
                      onChange={(e) => setEmail(e.target.value)}
                      maxLength={254}
                      required
                    />
                  </Field>
                )}
                {(link?.purpose === 'reset_password' || mode !== 'forgot') && (
                  <PasswordField
                    label={link ? t("Nové heslo") : t("Heslo")}
                    newPassword={!!link || mode === 'register'}
                  />
                )}
              </>
            )}
            <button
              className="button primary full-width"
              disabled={busy || (!link && mode === 'forgot' && !emailAvailable)}
            >
              {busy
                ? t("Zpracovávám…")
                : link
                  ? link.purpose === 'verify_email'
                    ? t("Ověřit e-mail")
                    : t("Uložit nové heslo")
                  : mode === 'register'
                    ? t("Vytvořit účet")
                    : mode === 'forgot'
                      ? t('Poslat odkaz na obnovu')
                      : t("Přihlásit se")}
              <ArrowRight size={19} aria-hidden="true" />
            </button>
          </fieldset>
        </form>
        {!link && (
          <div className="auth-links">
            {mode === 'login' ? (
              <>
                <button
                  disabled={busy}
                  onClick={() => {
                    setMode('register');
                    setError('');
                    setMessage('');
                  }}
                >{t("Ještě nemáte účet?")} <strong>{t("Zaregistrovat se")}</strong>
                </button>
                <button
                  disabled={busy}
                  onClick={() => {
                    setMode('forgot');
                    setError('');
                    setMessage('');
                  }}
                >{t("Zapomenuté heslo")} </button>
              </>
            ) : (
              <button
                disabled={busy}
                onClick={() => {
                  setMode('login');
                  setError('');
                  setMessage('');
                }}
              >{t("Zpět na přihlášení")} </button>
            )}
          </div>
        )}
        {!emailAvailable && mode === 'forgot' && (
          <p className="field-hint">{t("Odesílání e-mailů zatím není dostupné. Pokud jste přihlášení na jiném zařízení, můžete heslo změnit v nastavení.")} </p>
        )}
        <div className="auth-card-footnote"><ShieldCheck size={17} aria-hidden="true" /><span>{t('Test environment')}</span></div>
      </section>
    </div>
  );
}
