import { useEffect, useRef, useState } from 'react';
import { ArrowsOut, CaretDown, EyeSlash, TerminalWindow, Trash, X, ArrowSquareOut } from '@phosphor-icons/react';
import { useLanguage } from './i18n';
import { clearProcess, useProcess } from './process';
import './console.css';

export function ProcessConsole() {
  const language = useLanguage();
  const p = (en: string, cs: string) => language === 'en' ? en : cs;
  const events = useProcess();
  const [mode, setMode] = useState<'compact' | 'open' | 'hidden'>('compact');
  const [follow, setFollow] = useState(true);
  const dialog = useRef<HTMLDialogElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const bigList = useRef<HTMLDivElement>(null);
  function expand() { dialog.current?.showModal(); if (follow && bigList.current) bigList.current.scrollTop = bigList.current.scrollHeight; }
  useEffect(() => { if (follow) { for (const ref of [list, bigList]) if (ref.current) ref.current.scrollTop = ref.current.scrollHeight; } }, [events, follow, mode]);
  const log = (expanded: boolean) => <div className="console-log" ref={expanded ? bigList : list} role="log" aria-live="polite" aria-relevant="additions" aria-label={p('Observed process events', 'Pozorované události procesu')} tabIndex={0}>
    {events.length === 0 ? <div className="console-empty"><TerminalWindow size={30} aria-hidden="true" /><strong>{p('Ready to follow your next action.', 'Připraveno sledovat další akci.')}</strong><p>{p('Actual API responses, wallet steps and verified transaction states appear here. No simulated log entries.', 'Zde se zobrazí skutečné odpovědi API, kroky peněženky a ověřené stavy transakcí. Bez simulovaných výpisů.')}</p></div> : events.map(event => <div className={`console-entry console-${event.level}`} key={event.id}>
      <time dateTime={event.at}>{new Intl.DateTimeFormat(language === 'en' ? 'en-GB' : 'cs-CZ', { hour: '2-digit', minute: '2-digit', second: '2-digit' }).format(new Date(event.at))}</time>
      <span className="console-category">{event.category.toUpperCase()}</span>
      <span>{language === 'en' ? event.en : event.cs}{event.signature && <a href={`https://explorer.solana.com/tx/${encodeURIComponent(event.signature)}?cluster=devnet`} target="_blank" rel="noreferrer">{event.signature.slice(0, 9)}…{event.signature.slice(-5)} <ArrowSquareOut size={12} aria-hidden="true" /></a>}</span>
    </div>)}
  </div>;
  const controls = <div className="console-toolbar"><label><input type="checkbox" checked={follow} onChange={e => setFollow(e.target.checked)} />{p('Follow latest', 'Sledovat nejnovější')}</label><button onClick={clearProcess} aria-label={p('Clear process log', 'Vymazat výpis procesu')}><Trash size={16} aria-hidden="true" /></button></div>;
  return <>
    {mode === 'hidden' ? <button className="console-reopen" onClick={() => setMode('open')}><TerminalWindow size={20} aria-hidden="true" />{p('Live process', 'Živý průběh')}<span>{events.length}</span></button> : <aside className={`process-console console-${mode}`} aria-label={p('Live process console', 'Konzole živého průběhu')}>
      <div className="console-title"><button className="console-toggle" onClick={() => setMode(mode === 'open' ? 'compact' : 'open')} aria-expanded={mode === 'open'}><TerminalWindow size={19} aria-hidden="true" /><strong>{p('Live process', 'Živý průběh')}</strong><span className="console-count">{events.length}</span><CaretDown className={mode === 'open' ? 'rotated' : ''} size={15} aria-hidden="true" /></button><button onClick={expand} aria-label={p('Expand process console', 'Zvětšit konzoli průběhu')}><ArrowsOut size={18} aria-hidden="true" /></button><button onClick={() => setMode('hidden')} aria-label={p('Hide process console', 'Skrýt konzoli průběhu')}><EyeSlash size={18} aria-hidden="true" /></button></div>
      {mode === 'open' && <><p className="console-disclaimer">{p('Browser observations · API results · Devnet confirmations', 'Pozorování prohlížeče · výsledky API · potvrzení Devnetu')}</p>{log(false)}{controls}</>}
    </aside>}
    <dialog className="console-dialog" ref={dialog} aria-label={p('Expanded live process console', 'Zvětšená konzole živého průběhu')}><div className="console-title"><div><TerminalWindow size={23} aria-hidden="true" /><strong>{p('TruHabit / Live process', 'TruHabit / Živý průběh')}</strong></div><button onClick={() => dialog.current?.close()} autoFocus aria-label={p('Close expanded console', 'Zavřít zvětšenou konzoli')}><X size={22} aria-hidden="true" /></button></div><p className="console-disclaimer">{p('Real observed events. Request completion is not proof of blockchain finality; confirmed states are shown separately. Test funds only.', 'Skutečné pozorované události. Dokončení požadavku neprokazuje finalitu blockchainu; potvrzené stavy se zobrazují samostatně. Jen testovací prostředky.')}</p>{log(true)}{controls}</dialog>
  </>;
}
