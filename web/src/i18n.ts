import { useSyncExternalStore } from 'react';
import { messages } from './messages';

export type Language = 'en' | 'cs';
let language: Language = 'en';
try { if (localStorage.getItem('truhabit.language') === 'cs') language = 'cs'; } catch { /* Optional preference. */ }
const listeners = new Set<() => void>();
const czech = new Map(Object.entries(messages).map(([cs, en]) => [en, cs]));
export const getLanguage = () => language;
export const locale = () => language === 'cs' ? 'cs-CZ' : 'en-GB';
export function t(text: string): string {
  return language === 'en' ? (messages[text] ?? text) : (czech.get(text) ?? text);
}
export function setLanguage(value: Language) {
  language = value;
  document.documentElement.lang = value;
  try { localStorage.setItem('truhabit.language', value); } catch { /* Still works for this session. */ }
  listeners.forEach(listener => listener());
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
export function useLanguage() { return useSyncExternalStore(subscribe, getLanguage, () => 'en' as Language); }
