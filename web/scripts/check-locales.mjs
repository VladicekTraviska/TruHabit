import { readFileSync, readdirSync } from 'node:fs';
import { messages } from '../src/messages.ts';

// Node 24 type stripping; uses the same catalog as the app, no external packages.
const english = new Set(Object.values(messages));
const missing = new Set();
const source = new URL('../src/', import.meta.url);
for (const file of readdirSync(source).filter(name => name.endsWith('.tsx'))) {
  for (const match of readFileSync(new URL(file, source), 'utf8').matchAll(/\bt\(['"]([^'"\n]+)['"]\)/g)) {
    if (!messages[match[1]] && !english.has(match[1])) missing.add(`${file}: ${match[1]}`);
  }
}
for (const module of ['auth','wallet','goals','organizations','error','lib']) {
  const path = new URL(`../../crates/api/src/${module}.rs`, import.meta.url);
  for (const match of readFileSync(path, 'utf8').matchAll(/"([^"\n]*[áčďéěíňóřšťúůýž][^"\n]*)"/g)) {
    if (!messages[match[1]]) missing.add(`${module}.rs: ${match[1]}`);
  }
}
if (missing.size) {
  console.error('Missing EN/CS translations:\n' + [...missing].join('\n'));
  process.exitCode = 1;
} else console.log(`EN/CS catalog checked: ${Object.keys(messages).length} messages; UI literal keys and API messages covered.`);
