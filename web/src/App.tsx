import { useEffect, useRef, useState, type MouseEvent } from 'react';
import { Buildings, Flag, Globe, SignOut, Target, UserCircle, CaretRight, ShieldCheck } from '@phosphor-icons/react';
import { api, ApiError, logoutSession, setCsrf } from './api';
import type { Readiness, Session } from './types';
import { Brand, Message } from './components';
import { AuthPanel, readActionLink } from './AuthPanel';
import { Goals } from './Goals';
import { Account } from './Account';
import { Business } from './Business';
import { Prototype } from './Prototype';
import { ProcessConsole } from './ProcessConsole';
import { ProductHelp } from './ProductHelp';
import { resetProcessSession } from './process';
import { t, useLanguage, setLanguage } from './i18n';

type Page = 'goals' | 'account' | 'business' | 'prototype';
function readPage(): Page { const value = new URLSearchParams(location.search).get('view'); return value === 'goals' || value === 'account' || value === 'business' ? value : 'prototype'; }
export function App() {
  const language = useLanguage();
  const p = (en: string, cs: string) => language === 'en' ? en : cs;
  const [session, setSession] = useState<Session | null>(null);
  const [readiness, setReadiness] = useState<Readiness | null>(null);
  const [loading, setLoading] = useState(true);
  const [loggingOut, setLoggingOut] = useState(false);
  const [error, setError] = useState('');
  const [page, setPage] = useState<Page>(readPage);
  const [link, setLink] = useState(readActionLink);
  const [dataRevision, setDataRevision] = useState(0);
  const main = useRef<HTMLElement>(null);
  const previousPage = useRef(page);
  const mounted = useRef(true);
  const loadGeneration = useRef(0);
  const currentSession = useRef<Session | null>(session);
  currentSession.current = session;
  async function load() {
    if (!mounted.current) return;
    const generation = ++loadGeneration.current;
    const active = () => mounted.current && generation === loadGeneration.current;
    setError('');
    try {
      const ready = await api<Readiness>('/api/readiness');
      if (!active()) return;
      setReadiness(ready);
      try {
        const value = await api<Session>('/api/auth/session');
        if (!active()) return;
        if (currentSession.current && (currentSession.current.user.id !== value.user.id || currentSession.current.csrf_token !== value.csrf_token)) resetProcessSession();
        currentSession.current = value;
        setSession(value); setCsrf(value.csrf_token);
      } catch (e) {
        if (!active()) return;
        if (e instanceof ApiError && e.status === 401) { currentSession.current = null; setSession(null); setCsrf(''); resetProcessSession(); }
        else throw e;
      }
    } catch (e) { if (active()) setError(e instanceof Error ? e.message : t('Aplikace není dostupná.')); }
    finally { if (active()) setLoading(false); }
  }
  useEffect(() => {
    mounted.current = true;
    void load();
    const pop = () => setPage(readPage());
    const reconcileSession = () => { if (document.visibilityState === 'visible') void load(); };
    window.addEventListener('popstate', pop);
    window.addEventListener('focus', reconcileSession);
    document.addEventListener('visibilitychange', reconcileSession);
    return () => {
      mounted.current = false;
      loadGeneration.current++;
      window.removeEventListener('popstate', pop);
      window.removeEventListener('focus', reconcileSession);
      document.removeEventListener('visibilitychange', reconcileSession);
    };
  }, []);
  useEffect(() => { if (previousPage.current !== page && session) { main.current?.focus(); window.scrollTo({ top: 0, behavior: 'instant' }); } previousPage.current = page; }, [page, session]);
  function navigate(next: Page) { const url = new URL(location.href); url.searchParams.set('view', next); history.pushState(null, '', url.pathname + url.search); setPage(next); }
  function openPage(e: MouseEvent<HTMLAnchorElement>, next: Page) { if (e.ctrlKey || e.metaKey || e.shiftKey || e.altKey || e.button !== 0) return; e.preventDefault(); navigate(next); }
  function signedOut() { loadGeneration.current++; currentSession.current = null; setCsrf(''); resetProcessSession(); if (!mounted.current) return; setSession(null); setPage('prototype'); }
  async function profileReset(userId: string, csrfToken: string) {
    if (!mounted.current || currentSession.current?.user.id !== userId || currentSession.current?.csrf_token !== csrfToken) return;
    resetProcessSession();
    setDataRevision(value => value + 1);
    await load();
  }
  async function logout() {
    if (loggingOut || !mounted.current) return;
    const identity = currentSession.current?.user.id; const token = currentSession.current?.csrf_token;
    const active = () => mounted.current && currentSession.current?.user.id === identity && currentSession.current?.csrf_token === token;
    loadGeneration.current++;
    setLoggingOut(true);
    try { await logoutSession(); if (active()) signedOut(); }
    catch (e) { if (active()) setError(e instanceof Error ? e.message : t('Odhlášení se nezdařilo.')); }
    finally { if (mounted.current) setLoggingOut(false); }
  }
  const pages = [
    { id: 'prototype' as const, label: p('Challenges', 'Výzvy'), icon: Target },
    { id: 'goals' as const, label: p('My plans', 'Moje plány'), icon: Flag },
    { id: 'business' as const, label: p('For teams', 'Pro týmy'), icon: Buildings },
    { id: 'account' as const, label: p('My account', 'Můj účet'), icon: UserCircle },
  ];
  const signedIn = !!session && !link;
  const pageLabel = pages.find(item => item.id === page)?.label;
  const title = loading
    ? p('Loading…', 'Načítání…')
    : link
      ? link.purpose === 'verify_email' ? t('Potvrďte svůj e-mail') : t('Nové heslo')
      : signedIn ? pageLabel : p('Account access', 'Přístup k účtu');
  useEffect(() => { document.title = `TruHabit · ${title}`; }, [title]);
  const sessionKey = session ? `${session.user.id}:${session.csrf_token}:${dataRevision}` : '';
  const sessionEnded = () => {
    if (session && mounted.current && currentSession.current?.user.id === session.user.id && currentSession.current.csrf_token === session.csrf_token) signedOut();
  };
  return <div className={signedIn ? 'app-shell' : 'welcome-shell'}>
    <a className="skip-link" href="#main">{t('Přejít k obsahu')}</a>
    {signedIn && <aside className="app-sidebar"><Brand /><p className="sidebar-label">{p('YOUR SPACE', 'VÁŠ PROSTOR')}</p>
      <nav className="app-navigation" aria-label={t('Hlavní navigace')}>{pages.map(item => <a href={`/?view=${item.id}`} key={item.id} aria-current={page === item.id ? 'page' : undefined} onClick={e => openPage(e, item.id)}><item.icon size={22} weight={page === item.id ? 'duotone' : 'regular'} aria-hidden="true" /><span>{item.label}</span>{page === item.id && <span className="nav-indicator" aria-hidden="true" />}</a>)}</nav>
      <div className="sidebar-bottom"><div className="sidebar-promise"><ShieldCheck size={24} weight="duotone" aria-hidden="true" /><strong>{p('Small steps. Kept promises.', 'Malé kroky. Splněné sliby.')}</strong><p>{p('Your pace. Your commitment.', 'Vaše tempo. Váš závazek.')}</p></div><button className="sidebar-user" onClick={() => navigate('account')}><span className="avatar">{session.user.display_name.slice(0, 1).toUpperCase()}</span><span><strong>{session.user.display_name}</strong><small>{p('Personal account', 'Osobní účet')}</small></span><CaretRight size={16} aria-hidden="true" /></button></div>
    </aside>}
    <div className="app-content">
      <header className="app-topbar"><div className="topbar-context">{signedIn ? <><span className="topbar-brand" translate="no">TruHabit</span><CaretRight size={13} aria-hidden="true" /><span>{pageLabel}</span></> : <Brand />}</div><div className="topbar-actions">{signedIn&&<ProductHelp view={page}/>}<span className="environment-chip"><span aria-hidden="true" />{p('Test environment', 'Testovací prostředí')}</span><label className="language-select"><Globe size={18} aria-hidden="true" /><span className="sr-only">{p('Language', 'Jazyk')}</span><select value={language} onChange={e => setLanguage(e.target.value === 'cs' ? 'cs' : 'en')}><option value="en">English</option><option value="cs">Čeština</option></select></label></div></header>
      <main ref={main} className="wrap product-main" id="main" tabIndex={-1}>
        {error && <Message error><strong>{t('Aplikace není dostupná.')}</strong><span>{t(error)}</span><button className="button secondary" onClick={() => void load()}>{t('Zkusit znovu')}</button></Message>}
        {loading ? <div className="page-skeleton" role="status" aria-label={t('Načítám váš účet…')}><span>{t('Načítám váš účet…')}</span><div /><div /><div /></div> : !signedIn ? <AuthPanel onLogin={load} link={link} onDismissLink={() => setLink(null)} emailAvailable={readiness?.email_delivery ?? false} /> : readiness && <>
          {page === 'prototype' ? <Prototype key={sessionKey} session={session} onExpired={sessionEnded} onAccount={() => navigate('account')} /> : page === 'goals' ? <Goals key={sessionKey} session={session} readiness={readiness} onExpired={sessionEnded} /> : page === 'business' ? <Business key={sessionKey} session={session} onExpired={sessionEnded} /> : <Account key={sessionKey} session={session} readiness={readiness} refresh={load} onSignedOut={sessionEnded} onDataReset={profileReset} />}
          <div className="session-footer"><span><ShieldCheck size={15} aria-hidden="true" />{p('Private workspace', 'Soukromý prostor')}</span><button onClick={() => void logout()} disabled={loggingOut}><SignOut size={17} aria-hidden="true" />{loggingOut ? p('Signing out…', 'Odhlašuji…') : t('Odhlásit se')}</button></div>
        </>}
      </main>
      <footer className="wrap footer"><span translate="no">TruHabit <span className="muted">/</span><span translate="yes">{t('Důvěra začíná jasnými pravidly.')}</span></span><span>{t('Vaše cíle. Vaše tempo.')}</span></footer>
    </div>
    {signedIn && <ProcessConsole />}
  </div>;
}
