import { getLanguage, t } from './i18n';
import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { api, ApiError } from './api';
import type { Readiness, Session } from './types';
import { date } from './types';
import { Field, Message, PasswordField } from './components';
import { UserCircle, Wallet, ShieldCheck, DownloadSimple, SignOut, Trash, CaretDown, CaretRight, CheckCircle, LockKey } from '@phosphor-icons/react';
import './sections-ui.css';
import './auth-account.css';
import { DeveloperReset } from './DeveloperReset';

interface SolanaProvider {
  signTransaction?: (transaction: import('@solana/web3.js').Transaction) => Promise<import('@solana/web3.js').Transaction>;
  isPhantom?: boolean;
  publicKey?: { toString: () => string } | null;
  on?: (event: 'accountChanged' | 'disconnect', listener: () => void) => void;
  removeListener?: (event: 'accountChanged' | 'disconnect', listener: () => void) => void;
  connect: () => Promise<{ publicKey: { toString: () => string } }>;
  signMessage: (
    message: Uint8Array,
    encoding: 'utf8',
  ) => Promise<{ signature: Uint8Array; publicKey?: { toString: () => string } }>;
}
declare global {
  interface Window {
    phantom?: { solana?: SolanaProvider };
    solana?: SolanaProvider;
  }
}

export function Account({
  session,
  readiness,
  refresh,
  onSignedOut,
  onDataReset,
}: {
  session: Session;
  readiness: Readiness;
  refresh: () => Promise<void>;
  onSignedOut: () => void;
  onDataReset: (userId: string, csrfToken: string) => Promise<void>;
}) {
  const p = (en: string, cs: string) => getLanguage() === 'en' ? en : cs;
  const [accountBusy, setBusy] = useState(false);
  const [resetBusy, setResetBusy] = useState(false);
  const resetInFlight = useRef(false);
  const busy = accountBusy || resetBusy;
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [deleting, setDeleting] = useState(false);
  const mounted = useRef(true);
  const operationLock = useRef(false);
  const walletGeneration = useRef(0);
  const currentSession = useRef(session);
  currentSession.current = session;
  useEffect(() => {
    mounted.current = true;
    const provider = window.phantom?.solana ?? window.solana;
    const changed = () => { walletGeneration.current++; };
    if (provider?.isPhantom && provider.on && provider.removeListener) {
      provider.on('accountChanged', changed);
      provider.on('disconnect', changed);
    }
    return () => {
      mounted.current = false;
      walletGeneration.current++;
      provider?.removeListener?.('accountChanged', changed);
      provider?.removeListener?.('disconnect', changed);
    };
  }, []);
  async function run(action: () => Promise<void>) {
    if (operationLock.current || resetInFlight.current || !mounted.current) return;
    operationLock.current = true;
    const identity = session.user.id;
    const token = session.csrf_token;
    const active = () => mounted.current && currentSession.current.user.id === identity && currentSession.current.csrf_token === token;
    setBusy(true);
    setError('');
    setNotice('');
    try {
      await action();
    } catch (e) {
      if (active()) {
        setError(e instanceof ApiError && e.message === 'ACCOUNT_HAS_SHARED_CREDIT_HISTORY'
          ? t('Účet má společnou historii firemních odměn. Smazání by poškodilo záznamy ostatních účastníků. Své údaje můžete stáhnout; vývojářský reset dovolí vyčistit jen vlastní nesdílená demo data.')
          : e instanceof Error ? e.message : t("Operace se nepodařila."));
        if (e instanceof ApiError && e.code === 'UNAUTHORIZED') onSignedOut();
      }
    } finally {
      operationLock.current = false;
      if (active()) setBusy(false);
    }
  }
  function profile(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const form = new FormData(e.currentTarget);
    void run(async () => {
      await api('/api/account', {
        method: 'PATCH',
        body: { display_name: String(form.get('display_name')) },
      });
      await refresh();
      setNotice(t("Jméno bylo uložené."));
    });
  }
  function resetBusyChanged(value: boolean) {
    resetInFlight.current = value;
    if (mounted.current) setResetBusy(value);
  }
  async function resetComplete() {
    try { sessionStorage.removeItem(`truhabit.goal.pending.${session.user.id}`); }
    catch { /* Optional browser storage does not determine server reset success. */ }
    if (mounted.current && currentSession.current.user.id === session.user.id && currentSession.current.csrf_token === session.csrf_token) {
      walletGeneration.current++;
      setDeleting(false);
      setError('');
      setNotice('');
    }
    await onDataReset(session.user.id, session.csrf_token);
  }
  function password(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const form = new FormData(e.currentTarget);
    void run(async () => {
      await api('/api/auth/password', {
        method: 'POST',
        body: {
          current_password: String(form.get('current_password')),
          new_password: String(form.get('new_password')),
        },
      });
      onSignedOut();
    });
  }
  async function connectWallet() {
    await run(async () => {
      const identity = session.user.id;
      const token = session.csrf_token;
      const linkedWallet = session.wallet;
      const active = () => mounted.current && currentSession.current.user.id === identity && currentSession.current.csrf_token === token;
      const provider = window.phantom?.solana ?? window.solana;
      if (!provider?.isPhantom)
        throw new Error(t("Pro propojení otevřete aplikaci v prohlížeči s peněženkou Phantom."));
      async function request<T>(operation: () => Promise<T>): Promise<T> {
        try { return await operation(); }
        catch (cause) {
          const code = typeof cause === 'object' && cause !== null && 'code' in cause ? Number(cause.code) : null;
          if (code === 4001) throw new Error(t("Požadavek v Phantomu byl odmítnut. Propojení se nezměnilo; můžete to zkusit znovu."));
          if (code === -32002) throw new Error(t("Phantom už má otevřený požadavek. Dokončete ho nebo zavřete a pak zkuste propojení znovu."));
          throw new Error(t("Phantom nevrátil použitelnou odpověď. Zkontrolujte rozšíření a zkuste propojení znovu."));
        }
      }
      const address = await request(async () => {
        const connected = await provider.connect();
        if (!connected?.publicKey || typeof connected.publicKey.toString !== 'function') throw new Error('INVALID_PHANTOM_RESPONSE');
        const publicKey = connected.publicKey.toString();
        if (!publicKey) throw new Error('INVALID_PHANTOM_RESPONSE');
        return publicKey;
      });
      if (!active()) return;
      const generation = walletGeneration.current;
      const unchanged = () => {
        if (generation !== walletGeneration.current || currentSession.current.wallet !== linkedWallet || provider.publicKey === null || provider.publicKey && provider.publicKey.toString() !== address)
          throw new Error(t("Peněženka se při podepisování změnila. Zkuste propojení znovu."));
      };
      unchanged();
      const challenge = await api<{ id: string; message: string }>('/api/wallet/challenge', {
        method: 'POST',
        body: { public_key: address },
      });
      if (!active()) return;
      unchanged();
      const signed = await request(() => provider.signMessage(
        new TextEncoder().encode(challenge.message),
        'utf8',
      ));
      if (!active()) return;
      unchanged();
      if (!(signed?.signature instanceof Uint8Array) || signed.signature.length !== 64)
        throw new Error(t("Phantom nevrátil použitelnou odpověď. Zkontrolujte rozšíření a zkuste propojení znovu."));
      if (signed.publicKey && signed.publicKey.toString() !== address)
        throw new Error(t("Peněženka se při podepisování změnila. Zkuste propojení znovu."));
      const signature = btoa(String.fromCharCode(...signed.signature));
      await api('/api/wallet/link', {
        method: 'POST',
        body: { challenge_id: challenge.id, signature },
      });
      if (!active()) return;
      await refresh();
      if (active()) setNotice(t("Vlastnictví peněženky bylo ověřené. Žádné peníze se nepřevedly."));
    });
  }
  async function exportAccount() {
    await run(async () => {
      const data = await api<unknown>('/api/account/export');
      const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `truhabit-export-${new Date().toISOString().slice(0, 10)}.json`;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
      setNotice(t("Export vašich údajů je připravený ke stažení."));
    });
  }
  function deleteAccount(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const form = new FormData(e.currentTarget);
    void run(async () => {
      await api('/api/account', {
        method: 'DELETE',
        body: {
          password: String(form.get('password')),
          confirmation: String(form.get('confirmation')),
        },
      });
      onSignedOut();
    });
  }
  return (
    <div className="account-page sections-page" aria-busy={busy}>
      <section className="product-heading">
        <div>
          <p className="eyebrow">{t("VAŠE ÚDAJE, VAŠE KONTROLA")}</p>
          <h1>{t("Můj účet")}<span>.</span>
          </h1>
          <p className="lead">{t("Nastavení profilu, přístupu a peněženky na jednom místě.")}</p>
        </div>
      </section>
      {error && <Message error>{error}</Message>}
      {notice && <Message>{notice}</Message>}
      <div className="account-identity">
        <span className="identity-avatar" aria-hidden="true">{session.user.display_name.slice(0, 1).toUpperCase()}</span>
        <div><strong>{session.user.display_name}</strong><span>{session.user.email}</span><small>{t("Účet vytvořen")} {date(session.user.created_at)}</small></div>
        <span className={`badge ${session.user.email_verified_at ? 'success' : ''}`}><CheckCircle size={15} aria-hidden="true" />{session.user.email_verified_at ? t('E-mail ověřený') : t('E-mail zatím neověřený')}</span>
      </div>
      <div className="account-settings-layout">
      <nav className="account-section-index" aria-label={p('Account settings', 'Nastavení účtu')}>
        <p className="eyebrow">{p('SETTINGS', 'NASTAVENÍ')}</p>
        <a href="#account-profile"><UserCircle size={20} aria-hidden="true" /><span>{t('Profil')}</span><CaretRight size={15} aria-hidden="true" /></a>
        <a href="#account-wallet"><Wallet size={20} aria-hidden="true" /><span>{p('Wallet', 'Peněženka')}</span><CaretRight size={15} aria-hidden="true" /></a>
        <a href="#account-security"><LockKey size={20} aria-hidden="true" /><span>{p('Security', 'Zabezpečení')}</span><CaretRight size={15} aria-hidden="true" /></a>
        <a href="#account-privacy"><ShieldCheck size={20} aria-hidden="true" /><span>{p('Privacy & data', 'Soukromí a data')}</span><CaretRight size={15} aria-hidden="true" /></a>
        <div className="account-index-note"><ShieldCheck size={20} aria-hidden="true" /><p>{p('Your account. Your information. Your choice.', 'Váš účet. Vaše údaje. Vaše rozhodnutí.')}</p></div>
      </nav>
      <div className="account-settings-content">
      <div className="settings-grid">
        <section id="account-profile" className="panel settings-card" tabIndex={-1}>
          <div className="settings-card-heading"><span className="section-icon"><UserCircle size={23} aria-hidden="true" /></span><div><p className="eyebrow">{t("PROFIL")}</p><h2>{p('Your profile details', 'Údaje vašeho profilu')}</h2></div></div>
          <form onSubmit={profile}>
            <fieldset disabled={busy}>
              <div className="account-profile-fields">
              <Field label={t("Jméno")}>
                <input
                  name="display_name"
                  defaultValue={session.user.display_name}
                  maxLength={80}
                  autoComplete="nickname"
                  required
                />
              </Field>
              <Field label={t("E-mail")}>
                <input value={session.user.email} readOnly type="email" />
              </Field>
              </div>
              <div className="email-state">
                {!session.user.email_verified_at && (
                  <button
                    type="button"
                    className="link-button"
                    disabled={!readiness.email_delivery || busy}
                    onClick={() =>
                      void run(async () => {
                        await api('/api/auth/resend-verification', { method: 'POST', body: {} });
                        setNotice(t("Ověřovací e-mail byl zařazený k odeslání."));
                      })
                    }
                  >{t("Poslat ověřovací e-mail")} </button>
                )}
              </div>
              {!readiness.email_delivery && !session.user.email_verified_at && (
                <p className="field-hint">{t("Odesílání ověřovacích e-mailů zatím není dostupné.")}</p>
              )}
              <button className="button primary" disabled={busy}>{busy ? t('Ukládám…') : t("Uložit profil")} </button>
            </fieldset>
          </form>
        </section>
        <section id="account-wallet" className="panel settings-card" tabIndex={-1}>
          <div className="settings-card-heading"><span className="section-icon"><Wallet size={23} aria-hidden="true" /></span><div><p className="eyebrow">{t("SOLANA PENĚŽENKA")}</p><h2>{t("Vaše adresa. Váš podpis.")}</h2></div></div>
          <div className="account-wallet-context"><span className="badge">{p('Optional', 'Volitelné')}</span><p>{p('Use Phantom for personal Devnet challenges. Team programs work without a wallet.', 'Phantom využijete pro osobní výzvy na Devnetu. Týmové programy fungují bez peněženky.')}</p></div>
          <p className="body-copy">{t("Propojení ověří, že adresu ovládáte. Podepisujete jednorázovou zprávu pro tento účet, nikoli převod peněz.")} </p>
          {session.wallet ? (
            <>
              <div className="wallet-address">{session.wallet}</div>
              <span className="badge success"><CheckCircle size={15} aria-hidden="true" />{t("Vlastnictví ověřeno")}</span>
              <button
                className="button secondary"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await api('/api/wallet', { method: 'DELETE', body: {} });
                    await refresh();
                    setNotice(t("Peněženka byla odpojená."));
                  })
                }
              >{t("Odpojit peněženku")} </button>
            </>
          ) : (
            <button className="button primary" disabled={busy} onClick={() => void connectWallet()}>{t("Propojit Phantom")} </button>
          )}
          <p className="field-hint">{t("Propojení nebo odpojení vyžaduje přihlášení v posledních 15 minutách. Skutečné platby nejsou zapnuté.")} </p>
        </section>
        <section id="account-security" className="panel settings-card" tabIndex={-1}>
          <div className="settings-card-heading"><span className="section-icon"><ShieldCheck size={23} aria-hidden="true" /></span><div><p className="eyebrow">{t("ZABEZPEČENÍ")}</p><h2>{t('Security & access')}</h2></div></div>
          <details className="section-disclosure">
          <summary><span>{t('Change your password')}</span><CaretDown size={17} aria-hidden="true" /></summary>
          <div className="disclosure-content">
          <form onSubmit={password}>
            <input
              type="hidden"
              name="username"
              autoComplete="username"
              value={session.user.email}
            />
            <fieldset disabled={busy}>
              <PasswordField label={t("Současné heslo")} name="current_password" />
              <PasswordField label={t("Nové heslo")} name="new_password" newPassword />
              <p className="field-hint">{t("Po změně hesla se ukončí všechna přihlášení, včetně tohoto.")} </p>
              <button className="button primary" disabled={busy}>{t("Změnit heslo a odhlásit se")} </button>
            </fieldset>
          </form>
          </div>
          </details>
          <div className="settings-action-row"><div><strong>{p('Active sessions', 'Aktivní přihlášení')}</strong><p>{p('End every sign-in, including this device.', 'Ukončí všechna přihlášení, včetně tohoto zařízení.')}</p></div><button className="button secondary" disabled={busy} onClick={() => void run(async () => { await api('/api/auth/logout-all', { method: 'POST', body: {} }); onSignedOut(); })}><SignOut size={18} aria-hidden="true" />{t('Odhlásit všechna zařízení')}</button></div>
        </section>
        <section id="account-privacy" className="panel settings-card" tabIndex={-1}>
          <div className="settings-card-heading"><span className="section-icon"><DownloadSimple size={23} aria-hidden="true" /></span><div><p className="eyebrow">{t("PŘÍSTUP A SOUKROMÍ")}</p><h2>{t("Mějte účet pod kontrolou.")}</h2></div></div>
          <p className="body-copy">{t("Export obsahuje údaje vašeho účtu, cíle, jejich historii, propojenou adresu peněženky a záznamy zabezpečení.")} </p>
          <div className="stack-actions">
            <button
              className="button secondary"
              disabled={busy}
              onClick={() => void exportAccount()}
            ><DownloadSimple size={18} aria-hidden="true" />{t("Stáhnout moje údaje")} </button>
          </div>
          <details className="section-disclosure destructive-disclosure">
            <summary><span><Trash size={17} aria-hidden="true" />{t("Smazání účtu")}</span><CaretDown size={17} aria-hidden="true" /></summary>
            <div className="disclosure-content">
            <p className="body-copy">{t("Trvale odstraní účet, uložené cíle a propojení peněženky z aktivní databáze. Zálohy mohou údaje uchovat do konce své retenční doby.")} </p>
            {!deleting ? (
              <button
                className="link-button danger-link"
                disabled={busy}
                onClick={() => setDeleting(true)}
              >{t("Chci smazat účet")} </button>
            ) : (
              <form onSubmit={deleteAccount}>
                <input
                  type="hidden"
                  name="username"
                  autoComplete="username"
                  value={session.user.email}
                />
                <fieldset disabled={busy}>
                  <PasswordField label={t("Potvrďte současným heslem")} />
                  <Field label={t("Pro potvrzení napište SMAZAT")}>
                    <input name="confirmation" autoComplete="off" pattern={t("SMAZAT")} required />
                  </Field>
                  <button className="button danger" disabled={busy}>{t("Trvale smazat můj účet")} </button>
                  <button
                    className="button text-button"
                    type="button"
                    onClick={() => setDeleting(false)}
                  >{t("Zrušit")} </button>
                </fieldset>
              </form>
            )}
          </div>
          </details>
        </section>
      </div>
      {readiness.dev_reset && <DeveloperReset key={session.user.id} session={session} disabled={accountBusy} onReset={resetComplete} onBusyChange={resetBusyChanged} onExpired={onSignedOut} />}
      </div>
      </div>
    </div>
  );
}
