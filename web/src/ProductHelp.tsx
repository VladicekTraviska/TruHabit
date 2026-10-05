import { useEffect, useId, useRef, useState } from 'react';
import { CaretDown, Question, X } from '@phosphor-icons/react';
import { useLanguage } from './i18n';
import './product-help.css';

export type HelpView = 'prototype' | 'goals' | 'business' | 'account';
type HelpAction = { label: string; effect: string };

export function ProductHelp({ view }: { view: HelpView }) {
  const language = useLanguage();
  const p = (en: string, cs: string) => language === 'en' ? en : cs;
  const id = useId();
  const dialog = useRef<HTMLDialogElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => { if (dialog.current?.open) dialog.current.close(); }, [view]);

  function show() {
    if (!dialog.current || dialog.current.open) return;
    dialog.current.showModal();
    setOpen(true);
    closeButton.current?.focus();
  }
  function closed() {
    setOpen(false);
    if (trigger.current?.isConnected) trigger.current.focus();
  }

  const viewNames: Record<HelpView, string> = {
    prototype: p('Challenges', 'Výzvy'),
    goals: p('My plans', 'Moje plány'),
    business: p('For teams', 'Pro týmy'),
    account: p('My account', 'Můj účet'),
  };
  const actions: HelpAction[] = view === 'prototype' ? [
    { label: p('New challenge / Save challenge draft', 'Nová výzva / Uložit návrh výzvy'), effect: p('Choose the distance, stake and activity mode. Saving only creates a draft; it does not lock or transfer funds.', 'Zvolíte vzdálenost, vklad a režim aktivity. Uložení vytvoří pouze návrh; neuzamkne ani nepřevede prostředky.') },
    { label: p('Link Phantom', 'Propojit Phantom'), effect: p('In My account, sign a message to prove you control the wallet. This links the address; it transfers no tokens.', 'V Mém účtu podepíšete zprávu, že peněženku ovládáte. Tím propojíte adresu; žádné tokeny se nepřevedou.') },
    { label: p('Activate & lock stake', 'Aktivovat a uzamknout vklad'), effect: p('Locks the agreed amount. On Devnet, you approve the deposit transaction in Phantom. Wait for Active before uploading.', 'Uzamkne sjednanou částku. Na Devnetu potvrdíte transakci vkladu v Phantomu. Před nahráním počkejte na stav Aktivní.') },
    { label: p('Upload file & check goal', 'Nahrát soubor a ověřit cíl'), effect: p('Saves your GPX or FIT and checks the distance, times and recorded sensor data. Choosing a file alone does not submit it. Uploading does not return your stake.', 'Uloží vaše GPX nebo FIT a zkontroluje vzdálenost, časy a zaznamenaná data senzorů. Samotný výběr soubor neodešle. Nahrání ještě nevrací vklad.') },
    { label: p('Check latest status / Refresh wallet', 'Zkontrolovat aktuální stav / Obnovit peněženku'), effect: p('Reads the latest saved result and checks pending transactions, or reloads your wallet balance. Refreshing does not make a new deposit.', 'Načte poslední uložený výsledek a ověří čekající transakce, nebo znovu načte zůstatek peněženky. Obnovení nevytvoří nový vklad.') },
    { label: p('Return test stake', 'Vrátit testovací vklad'), effect: p('After an accepted run meets the goal, requests the return of the locked amount. Returned means the transfer has been confirmed.', 'Když přijatý běh splní cíl, požádá o vrácení uzamčené částky. Stav Vráceno znamená, že byl převod potvrzen.') },
    { label: p('Cancel & return stake', 'Zrušit a vrátit vklad'), effect: p('Before the agreed start time, cancels the active challenge and requests the return of its stake. It is unavailable once the run window starts.', 'Před sjednaným začátkem zruší aktivní výzvu a požádá o vrácení jejího vkladu. Po začátku časového okna už není dostupné.') },
    { label: p('Return unsettled stake', 'Vrátit nevypořádaný vklad'), effect: p('Available from the timeout return time shown under Your agreement. Requests a return if the stake is still unsettled. Reaching a deadline does not itself send tokens.', 'Je dostupné od času vrácení při timeoutu uvedeného ve Vaší dohodě. Požádá o vrácení dosud nevypořádaného vkladu. Samotné dosažení termínu tokeny neodešle.') },
    { label: p('Download original file / Remove source file', 'Stáhnout původní soubor / Smazat zdrojový soubor'), effect: p('Downloads the file you submitted. After settlement, you can remove that file; the recorded outcome and transfers remain.', 'Stáhne soubor, který jste nahráli. Po vypořádání lze tento soubor smazat; uložený výsledek a převody zůstávají.') },
    { label: p('Request a review', 'Požádat o kontrolu'), effect: p('After settlement, saves a request for a human response. It does not reopen the challenge or reverse a completed blockchain transfer.', 'Po vypořádání uloží žádost o odpověď člověka. Znovu neotevře výzvu ani nezruší dokončený blockchainový převod.') },
    { label: p('Live process', 'Živý průběh'), effect: p('Shows events observed during your actions. Open, enlarge or hide it as needed. Clear only clears this browser log; saved history and transactions stay. A successful request is not yet a confirmed transfer.', 'Ukazuje události zaznamenané při vašich akcích. Lze jej otevřít, zvětšit nebo skrýt. Vymazání odstraní pouze tento výpis v prohlížeči; historie a transakce zůstanou. Úspěšný požadavek ještě není potvrzený převod.') },
    { label: p('Operator controls', 'Ovládání správce'), effect: p('Only a designated reviewer sees manual decisions and failure settlement. A rejected activity alone does not forfeit the stake.', 'Ruční rozhodnutí a vypořádání neúspěchu vidí jen určený posuzovatel. Samotné odmítnutí aktivity vklad neodešle příjemci.') },
  ] : view === 'goals' ? [
    { label: p('New goal / Review plan', 'Nový cíl / Zkontrolovat plán'), effect: p('Choose a distance and schedule, then check the summary. Nothing is saved until you confirm.', 'Zvolíte vzdálenost a termín a projdete souhrn. Nic se neuloží, dokud uložení nepotvrdíte.') },
    { label: p('Save plan without a deposit', 'Uložit plán bez vkladu'), effect: p('Saves a private plan. The planned amount is a note, not a payment, locked stake or blockchain transaction.', 'Uloží soukromý plán. Plánovaná částka je údaj, nikoli platba, uzamčený vklad nebo blockchainová transakce.') },
    { label: p('Edit goal / Archive', 'Upravit cíl / Archivovat'), effect: p('Edit changes the saved plan. Archive moves it out of the Prepared list and keeps its record in Archive.', 'Úprava změní uložený plán. Archivace jej přesune ze seznamu Připravené a ponechá jeho záznam v Archivu.') },
    { label: p('Back / Cancel', 'Zpět / Zrušit'), effect: p('Returns to the previous form step or closes unsaved changes. It does not cancel an active challenge.', 'Vrátí vás na předchozí krok formuláře nebo zavře neuložené změny. Neruší aktivní výzvu.') },
  ] : view === 'business' ? [
    { label: p('New workspace / New program', 'Nový prostor / Nový program'), effect: p('Creates a private workspace or saves a program draft. Its CZK estimate is planning only.', 'Vytvoří soukromý prostor nebo uloží návrh programu. Odhad v Kč je pouze plán.') },
    { label: p('Fund & publish', 'Financovat a zveřejnit'), effect: p('The owner reserves LOCAL test credits and freezes the distance, reward and dates. No blockchain transfer or real payment is used.', 'Vlastník rezervuje LOCAL testovací kredity a zafixuje vzdálenost, odměnu a termíny. Bez blockchainového převodu a skutečné platby.') },
    { label: p('Invite / Accept invitation', 'Pozvat / Přijmout pozvánku'), effect: p('Create a private one-time code and share it manually with its recipient. The recipient signs in with the matching email and accepts. No email is sent by the app.', 'Vytvořte soukromý jednorázový kód a ručně jej předejte příjemci. Ten se přihlásí odpovídajícím e-mailem a pozvánku přijme. Aplikace neposílá e-mail.') },
    { label: p('Join / Upload & check run', 'Přihlásit se / Nahrát a zkontrolovat běh'), effect: p('Voluntarily reserves a funded reward place, then checks your private GPX/FIT. Employees do not stake or lose their own credits. Employers see status, not GPS or biometry.', 'Dobrovolně rezervuje místo s financovanou odměnou a potom zkontroluje soukromý GPX/FIT. Zaměstnanci nevkládají ani neztrácejí vlastní kredity. Firma vidí stav, ne GPS nebo biometrii.') },
    { label: p('Claim reward / Close program', 'Vyzvednout odměnu / Uzavřít program'), effect: p('A qualifying participant receives the LOCAL reward once. Closure returns unused credits to the original funder and protects accepted unpaid rewards. Pending review is protected until its agreed deadline.', 'Vyhovující účastník získá LOCAL odměnu jednou. Uzavření vrací nevyužité kredity původnímu financujícímu účtu a chrání přijaté nevyplacené odměny. Čekající posouzení je chráněné do sjednaného termínu.') },
  ] : [
    { label: p('Save profile', 'Uložit profil'), effect: p('Changes the display name for your account. It does not change your wallet or challenge terms.', 'Změní zobrazované jméno vašeho účtu. Nemění peněženku ani podmínky výzev.') },
    { label: p('Link Phantom / Unlink wallet', 'Propojit Phantom / Odpojit peněženku'), effect: p('Linking asks Phantom to sign a one-time ownership message, not a token transfer. Unlinking removes the account link; it does not move tokens or settle a challenge. Recent sign-in is required.', 'Propojení požádá Phantom o podpis jednorázové zprávy o vlastnictví, nikoli o převod tokenů. Odpojení odstraní vazbu na účet; nepřevádí tokeny ani nevypořádá výzvu. Vyžaduje nedávné přihlášení.') },
    { label: p('Change password / Sign out all devices', 'Změnit heslo / Odhlásit všechna zařízení'), effect: p('Changing your password ends existing sessions and signs you out. Sign out all devices ends every session for this account.', 'Změna hesla ukončí dosavadní relace a odhlásí vás. Odhlášení všech zařízení ukončí každé přihlášení k tomuto účtu.') },
    { label: p('Send verification email', 'Poslat ověřovací e-mail'), effect: p('Sends an email confirmation link when email delivery is configured. It does not link a wallet.', 'Při nastaveném odesílání e-mailů pošle odkaz pro potvrzení e-mailové adresy. Nepropojuje peněženku.') },
    { label: p('Download my data / Delete account', 'Stáhnout moje údaje / Smazat účet'), effect: p('Download exports your account records. Deletion permanently removes your account after confirmation and requires active challenges to be settled first.', 'Stažení vyexportuje údaje účtu. Smazání po potvrzení trvale odstraní účet a vyžaduje nejprve vypořádat aktivní výzvy.') },
  ];

  const steps = [
    [p('Choose the right activity mode', 'Zvolte správný režim aktivity'), p('For an older Garmin file, choose Use a saved GPX/FIT. Record a new run requires a run within the agreed 24 hours.', 'Pro starší soubor z Garminu zvolte Použít starší GPX/FIT. Zaznamenat nový běh vyžaduje běh ve sjednaných 24 hodinách.')],
    [p('Save, then activate', 'Uložte a potom aktivujte'), p('A draft is only a plan. Activate & lock stake makes it active and locks your test stake.', 'Návrh je pouze plán. Aktivovat a uzamknout vklad z něj vytvoří aktivní výzvu a uzamkne testovací vklad.')],
    [p('Upload and read the result', 'Nahrajte běh a přečtěte výsledek'), p('Select GPX or FIT, then submit it. The result explains whether this run counts toward this challenge or needs review.', 'Vyberte GPX nebo FIT a odešlete jej. Výsledek vysvětlí, zda se běh započítá do této výzvy nebo potřebuje posouzení.')],
    [p('Request the return', 'Požádejte o vrácení'), p('When the goal is met, choose Return test stake. Wait for Returned; on Devnet, inspect the transfer in Recorded history.', 'Při splnění cíle zvolte Vrátit testovací vklad. Počkejte na stav Vráceno; na Devnetu ověřte převod v Zaznamenané historii.')],
  ];

  return <>
    <button type="button" className="product-help-trigger" ref={trigger} onClick={show} aria-haspopup="dialog" aria-expanded={open} aria-controls={id} aria-label={p('How this works', 'Jak to funguje')}>
      <Question size={20} aria-hidden="true" /><span>{p('How this works', 'Jak to funguje')}</span>
    </button>
    <dialog ref={dialog} id={id} className="product-help-dialog" aria-labelledby={id + '-title'} onClose={closed}>
      <header className="product-help-header"><div><span>{viewNames[view]}</span><h2 id={id + '-title'}>{p('How this works', 'Jak to funguje')}</h2></div><button type="button" ref={closeButton} onClick={() => dialog.current?.close()} aria-label={p('Close help', 'Zavřít nápovědu')}><X size={22} aria-hidden="true" /></button></header>
      <div className="product-help-body">
        {view === 'prototype' ? <>
          <div className="product-help-environments"><p><strong>{p('Local simulation', 'Lokální simulace')}</strong>{p('Credits stay in the app. No wallet or blockchain transfer is used.', 'Kredity zůstávají v aplikaci. Nepoužívá se peněženka ani blockchainový převod.')}</p><p><strong>Solana Devnet</strong>{p('Uses Phantom and real transactions with valueless TruHabit Test Tokens (THT). These are not USDC or real money.', 'Používá Phantom a skutečné transakce s bezcennými TruHabit Test Tokeny (THT). Nejde o USDC ani skutečné peníze.')}</p></div>
          <ol className="product-help-steps">{steps.map(([title, text], index) => <li key={index}><span aria-hidden="true">{index + 1}</span><div><strong>{title}</strong><p>{text}</p></div></li>)}</ol>
          <section className="product-help-result"><h3>{p('Why might a real run not count?', 'Proč se nemusí započítat skutečný běh?')}</h3><p>{p('It may meet the distance but fall outside this challenge’s dates. This does not mean the run was fake. Compare its start and end with Your agreement. Choose Use a saved GPX/FIT for an older file. A new draft does not change the original challenge or release its locked stake.', 'Může splnit vzdálenost, ale být mimo termín této výzvy. To neznamená, že byl běh falešný. Porovnejte jeho začátek a konec s Vaší dohodou. Pro starší soubor zvolte Použít starší GPX/FIT. Nový návrh nemění původní výzvu ani nevrací její uzamčený vklad.')}</p><p>{p('Needs review means a person must assess unusual measurements; the stake remains locked. Heart rate and cadence help check consistency, but an uploaded file cannot prove who ran.', 'K posouzení znamená, že neobvyklá měření musí posoudit člověk; vklad zůstává uzamčený. Tep a kadence pomáhají ověřit soulad dat, ale nahraný soubor neprokáže, kdo běžel.')}</p><p>{p('Saved-file mode allows older activity dates, but you still have 10 minutes after saving to activate and upload. An activity that already met another challenge’s goal cannot be reused.', 'Režim staršího souboru dovoluje dřívější datum aktivity, ale na aktivaci a nahrání máte stále 10 minut od uložení. Aktivitu, která už splnila cíl jiné výzvy, nelze použít znovu.')}</p></section>
        </> : <p className="product-help-intro">{view === 'goals' ? p('This page saves personal plans without deposits. To lock a test stake and upload a run, use Challenges.', 'Tato stránka ukládá osobní plány bez vkladu. Pro uzamčení testovacího vkladu a nahrání běhu použijte Výzvy.') : view === 'business' ? p('Create a team program, fund it with LOCAL test credits, invite members, verify private runs and pay test rewards. For personal blockchain challenges use Challenges.', 'Vytvořte týmový program, financujte jej LOCAL testovacími kredity, pozvěte členy, ověřte soukromé běhy a vyplaťte testovací odměny. Osobní blockchainové výzvy jsou ve Výzvách.') : p('Manage your profile, wallet link, password and private account data. Linking Phantom proves ownership of an address; the separate deposit happens only when you activate a Devnet challenge.', 'Spravujte profil, propojení peněženky, heslo a soukromé údaje účtu. Propojení Phantomu ověří vlastnictví adresy; samostatný vklad proběhne až při aktivaci Devnet výzvy.')}</p>}
        <details className="product-help-actions" open={view !== 'prototype'}><summary><strong>{p('What each button does', 'Co dělá které tlačítko')}</strong><CaretDown size={18} aria-hidden="true" /></summary><table><thead><tr><th scope="col">{p('Action', 'Akce')}</th><th scope="col">{p('What happens', 'Co se stane')}</th></tr></thead><tbody>{actions.map(action => <tr key={action.label}><th scope="row">{action.label}</th><td>{action.effect}</td></tr>)}</tbody></table></details>
      </div>
    </dialog>
  </>;
}
