import { getLanguage, locale, t } from './i18n';
import { useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { api, ApiError, isAuthRetryBlocked, type AuthCooldown } from './api';
import { Field, PasswordField, Message } from './components';
import { ArrowLeft, ArrowRight, ShieldCheck, Target, Sneaker, CheckCircle } from '@phosphor-icons/react';
import './sections-ui.css';
import './auth-account.css';

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
  const p = (en: string, cs: string) => getLanguage() === 'en' ? en : cs;
  const [mode, setMode] = useState<'login' | 'register' | 'forgot'>('login');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const [email, setEmail] = useState('');
  const [accountExists, setAccountExists] = useState(false);
  const [cooldown, setCooldown] = useState<AuthCooldown | null>(null);
  const [now, setNow] = useState(Date.now);
  const action = link?.purpose ?? mode;
  const retryBlocked = isAuthRetryBlocked(cooldown, action, email, now);
  useEffect(() => {
    if (!cooldown) return;
    const timer = window.setInterval(() => {
      const current = Date.now();
      setNow(current);
      if (current >= cooldown.until) setCooldown(null);
    }, 1000);
    return () => window.clearInterval(timer);
  }, [cooldown]);
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy || retryBlocked) return;
    setBusy(true);
    setError('');
    setMessage('');
    setAccountExists(false);
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
      if (e instanceof ApiError) {
        setAccountExists(e.code === 'ACCOUNT_EXISTS');
        if (e.retryAfterSeconds !== null) {
          const current = Date.now();
          setNow(current);
          setCooldown({ action, email, until: current + e.retryAfterSeconds * 1000 });
        }
      }
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
      <section className="auth-story" aria-labelledby="auth-story-title">
        <p className="eyebrow">{t("PROSTOR PRO VAŠE CÍLE")}</p>
        <h1 id="auth-story-title">{t("Držte slovo.")} <br />
          <span>{t("Hlavně sobě.")}</span>
        </h1>
        <p className="auth-story-lead">{p('Set your goal. Record your run. Keep your commitment.', 'Stanovte si cíl. Zaznamenejte běh. Dodržte svůj slib.')}</p>
        <div className="auth-track" aria-hidden="true">
          <svg viewBox="0 0 650 400" fill="none" preserveAspectRatio="xMidYMax slice">
            <path className="track-lane" d="M-30 252C190 220 530 170 550 117S470 75 705 25" />
            <path className="track-lane" d="M-30 290C175 251 580 196 592 133S488 77 705 40" />
            <path className="track-lane" d="M15 455C-20 296 624 238 628 158S506 89 705 54" />
            <path className="track-lane" d="M91 455C48 317 652 265 663 183S524 100 705 67" />
            <path className="track-lane" d="M180 455C118 340 680 299 699 210S544 112 705 80" />
            <path className="track-progress" d="M91 455C69 382 225 331 367 297" />
            <circle className="track-halo" cx="367" cy="297" r="28" />
            <circle className="track-dot" cx="367" cy="297" r="10" />
          </svg>
        </div>
        <ol className="auth-points">
          <li><span className="auth-step-number">01</span><Target size={25} aria-hidden="true" /><div><strong>{p('Choose your goal', 'Vyberte si cíl')}</strong><p>{p('An achievable distance. A clear deadline.', 'Dosažitelná vzdálenost. Jasný termín.')}</p></div></li>
          <li><span className="auth-step-number">02</span><Sneaker size={25} aria-hidden="true" /><div><strong>{p('Record your activity', 'Zaznamenejte aktivitu')}</strong><p>{p('Submit a GPX or FIT file from your run.', 'Doložte běh souborem GPX nebo FIT.')}</p></div></li>
          <li><span className="auth-step-number">03</span><CheckCircle size={25} aria-hidden="true" /><div><strong>{p('See the outcome', 'Uvidíte výsledek')}</strong><p>{p('Check your progress, result and recorded history.', 'Sledujte průběh, výsledek i uloženou historii.')}</p></div></li>
        </ol>
        <div className="auth-availability"><ShieldCheck size={19} aria-hidden="true" /><p>{t("Prototyp používá pouze testovací prostředky. Běh můžete doložit souborem GPX nebo FIT. Skutečné platby nejsou zapnuté.")}</p></div>
      </section>
      <section className="panel auth-card" aria-labelledby="auth-title" aria-busy={busy}>
        <p className="eyebrow">{t("TRUHABIT ÚČET")}</p>
        <h2 id="auth-title">{title}</h2>
        <p className="auth-form-intro">{link ? link.purpose === 'verify_email' ? p('One last step to confirm your email address.', 'Poslední krok k potvrzení e-mailové adresy.') : p('Choose a new password for your account.', 'Zvolte nové heslo ke svému účtu.') : mode === 'register' ? p('Your goals and running records, in one private space.', 'Vaše cíle a běžecké záznamy na jednom soukromém místě.') : mode === 'forgot' ? emailAvailable ? p('We will send a recovery link to your email.', 'Na e-mail vám pošleme odkaz na obnovu přístupu.') : p('Email recovery is not enabled on this installation.', 'Obnova e-mailem není v této instalaci zapnutá.') : p('Sign in to your TruHabit account.', 'Přihlaste se ke svému účtu TruHabit.')}</p>
        {!link && mode !== 'forgot' && <div className="auth-mode-switch" aria-label={t('TRUHABIT ÚČET')}>
          <button type="button" aria-pressed={mode === 'login'} disabled={busy} onClick={() => { setMode('login'); setError(''); setMessage(''); }}>{t('Přihlásit se')}</button>
          <button type="button" aria-pressed={mode === 'register'} disabled={busy} onClick={() => { setMode('register'); setError(''); setMessage(''); }}>{t('Vytvořit účet')}</button>
        </div>}
        {error && <Message error>{error}{accountExists && mode === 'register' && <button type="button" className="button secondary" disabled={busy} onClick={() => { setMode('login'); setMessage(error); setError(''); }}>{t('Přihlásit se')}</button>}</Message>}
        {message && <Message>{message}</Message>}
        {retryBlocked && cooldown && <p className="field-hint" role="status">{t('Další pokus je možný od')} {new Intl.DateTimeFormat(locale(), { hour: '2-digit', minute: '2-digit', second: '2-digit' }).format(cooldown.until)}.</p>}
        <form className="auth-form" onSubmit={submit} key={`${mode}-${link?.purpose ?? ''}`}>
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
              disabled={busy || retryBlocked || (!link && mode === 'forgot' && !emailAvailable)}
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
                <button type="button"
                  disabled={busy}
                  onClick={() => {
                    setMode('forgot');
                    setError('');
                    setMessage('');
                  }}
                >{t("Zapomenuté heslo")}</button>
            ) : (
              <button type="button"
                disabled={busy}
                onClick={() => {
                  setMode('login');
                  setError('');
                  setMessage('');
                }}
              ><ArrowLeft size={16} aria-hidden="true" />{t("Zpět na přihlášení")}</button>
            )}
          </div>
        )}
        {!emailAvailable && mode === 'forgot' && (
          <p className="field-hint">{t("Odesílání e-mailů zatím není dostupné. Pokud jste přihlášení na jiném zařízení, můžete heslo změnit v nastavení.")} </p>
        )}
        <div className="auth-card-footnote"><ShieldCheck size={17} aria-hidden="true" /><span>{p('Test funds only. Real payments are disabled.', 'Jen testovací prostředky. Skutečné platby nejsou zapnuté.')}</span></div>
      </section>
    </div>
  );
}
