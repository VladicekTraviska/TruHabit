import { t, locale } from './i18n';
import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { api, ApiError } from './api';
import { date, money } from './types';
import type { Goal, GoalEvent, GoalInput, GoalPage, Session, Readiness } from './types';
import { Message, Field } from './components';
import { Target, ArrowRight, CalendarBlank, Timer, Plus, PencilSimple, Archive, CaretDown, ShieldCheck, Check, WarningCircle } from '@phosphor-icons/react';
import './sections-ui.css';

type Pending = { key: string; body: GoalInput };
export function Goals({
  session,
  readiness,
  onExpired,
}: {
  session: Session;
  readiness: Readiness;
  onExpired: () => void;
}) {
  const [goals, setGoals] = useState<Goal[]>([]);
  const [next, setNext] = useState<string | null>(null);
  const [selected, setSelected] = useState<Goal | null>(null);
  const [events, setEvents] = useState<GoalEvent[]>([]);
  const [editing, setEditing] = useState<Goal | null>(null);
  const [creating, setCreating] = useState(false);
  const [formRevision, setFormRevision] = useState(0);
  const [loading, setLoading] = useState(true);
  const [moreLoading, setMoreLoading] = useState(false);
  const [detailLoading, setDetailLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [filter, setFilter] = useState<'DRAFT' | 'ARCHIVED'>('DRAFT');
  const storageKey = `truhabit.goal.pending.${session.user.id}`;
  const [pending, setPending] = useState<Pending | null>(() => {
    try {
      return JSON.parse(sessionStorage.getItem(storageKey) ?? 'null') as Pending | null;
    } catch {
      return null;
    }
  });
  const lock = useRef(false);
  const selectionRequest = useRef(0);
  function rememberPending(command: Pending | null) {
    setPending(command);
    try {
      if (command) sessionStorage.setItem(storageKey, JSON.stringify(command));
      else sessionStorage.removeItem(storageKey);
    } catch {
      // A blocked or full browser store must not prevent saving or retrying in this tab.
    }
  }
  function beginCreate() {
    setFormRevision(value => value + 1);
    setCreating(true);
    setEditing(null);
    setSelected(null);
    setDetailLoading(false);
    selectionRequest.current++;
  }
  function fail(e: unknown) {
    if (e instanceof ApiError && e.status === 401) {
      onExpired();
      return;
    }
    setError(e instanceof Error ? e.message : t("Operaci se nepodařilo dokončit."));
  }
  async function load(more = false) {
    if (more) setMoreLoading(true);
    try {
      const data = await api<GoalPage>(`/api/goals${more && next ? `?after=${next}` : ''}`);
      setGoals((old) => (more ? [...old, ...data.goals] : data.goals));
      setNext(data.next_cursor);
    } catch (e) {
      fail(e);
    } finally {
      setLoading(false);
      setMoreLoading(false);
    }
  }
  useEffect(() => {
    void load();
  }, []);
  async function detail(goal: Goal) {
    const request = ++selectionRequest.current;
    setSelected(goal);
    setEvents([]);
    setDetailLoading(true);
    setError('');
    setCreating(false);
    setEditing(null);
    try {
      const result = await api<{ goal: Goal; events: GoalEvent[] }>(`/api/goals/${goal.id}`);
      if (request === selectionRequest.current) {
        setSelected(result.goal);
        setEvents(result.events);
      }
    } catch (e) {
      if (request === selectionRequest.current) fail(e);
    } finally {
      if (request === selectionRequest.current) setDetailLoading(false);
    }
  }
  async function save(input: GoalInput, existing?: Goal, repeat?: Pending) {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    setError('');
    setNotice('');
    try {
      let result: Goal;
      if (existing) {
        result = await api<Goal>(`/api/goals/${existing.id}`, {
          method: 'PATCH',
          body: { ...input, version: existing.version },
        });
      } else {
        const command = repeat ?? { key: crypto.randomUUID(), body: input };
        rememberPending(command);
        try {
          result = await api<Goal>('/api/goals', {
            method: 'POST',
            body: command.body,
            key: command.key,
          });
        } catch (e) {
          if (e instanceof ApiError && e.status >= 400 && e.status < 500) {
            rememberPending(null);
          }
          throw e;
        }
        rememberPending(null);
      }
      setCreating(false);
      setEditing(null);
      await load();
      await detail(result);
      setFilter(result.state);
      setNotice(t("Cíl je uložený. Žádné peníze nebyly vložené."));
    } catch (e) {
      fail(e);
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  async function archive(goal: Goal) {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    setError('');
    try {
      const result = await api<Goal>(`/api/goals/${goal.id}/archive`, {
        method: 'POST',
        body: { version: goal.version },
      });
      await load();
      await detail(result);
      setFilter('ARCHIVED');
      setNotice(t("Cíl byl archivovaný."));
    } catch (e) {
      fail(e);
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  const visible = goals.filter((g) => g.state === filter);
  const disabled = busy || pending !== null;
  return (
    <div className="goals-page sections-page">
      <section className="product-heading">
        <div>
          <p className="eyebrow">{t("JEDEN KROK PO DRUHÉM")}</p>
          <h1>{t("Moje cíle")}</h1>
          <p className="lead">{t("Naplánujte běh, na který si uděláte čas.")}</p>
        </div>
        <button
          className="button primary"
          onClick={beginCreate}
          disabled={disabled}
        ><Plus size={19} aria-hidden="true" />{t('New goal')} </button>
      </section>
      <details className="availability-bar sections-availability"><summary><ShieldCheck size={20} aria-hidden="true" /><strong>{t("Cíle můžete připravovat bez vkladu.")}</strong><CaretDown size={17} aria-hidden="true" /></summary><p>{t(readiness.reason)}</p></details>
      {error && <Message error>{error}</Message>}
      {notice && <Message>{notice}</Message>}
      {pending && !busy && (
        <Message>
          <strong>{t("Uložení má nepotvrzený výsledek.")}</strong>
          <p>{t("Obnovíme stejný požadavek, aby nevznikl druhý cíl.")}</p>
          <button
            className="button secondary"
            onClick={() => void save(pending.body, undefined, pending)}
          >{t("Ověřit a dokončit uložení")} </button>
        </Message>
      )}
      {loading ? (
        <div className="panel section-loading" role="status"><span className="loading-spinner" aria-hidden="true" />{t("Načítám vaše cíle…")}</div>
      ) : creating || editing ? (
        <GoalForm
          key={editing?.id ?? `new-${formRevision}`}
          initial={editing}
          disabled={disabled}
          onSave={(input) => void save(input, editing ?? undefined)}
          onCancel={() => {
            setCreating(false);
            setEditing(null);
          }}
        />
      ) : (
        <>
          <div className="section-title">
            <h2>{t('Your personal space')}</h2>
            <div className="filter-buttons" aria-label={t("Filtr cílů")}>
              <button
                aria-pressed={filter === 'DRAFT'}
                disabled={busy}
                onClick={() => {
                  setFilter('DRAFT');
                  setSelected(null);
                  setDetailLoading(false);
                  selectionRequest.current++;
                }}
              >{t("Připravené")} </button>
              <button
                aria-pressed={filter === 'ARCHIVED'}
                disabled={busy}
                onClick={() => {
                  setFilter('ARCHIVED');
                  setSelected(null);
                  setDetailLoading(false);
                  selectionRequest.current++;
                }}
              >{t("Archiv")} </button>
            </div>
          </div>
          <div className="goals-layout">
            <section className="goal-cards" aria-label={t("Seznam cílů")}>
              {!visible.length ? (
                <div className="panel empty-state">
                  <span className="empty-symbol" aria-hidden="true">{error && !goals.length ? <WarningCircle size={30} weight="duotone" /> : <Target size={30} weight="duotone" />}</span>
                  <h3>
                    {error && !goals.length ? t('Operaci se nepodařilo dokončit.') : filter === 'DRAFT'
                      ? t("Každý návyk začíná jedním krokem.")
                      : t("Archiv je zatím prázdný.")}
                  </h3>
                  {!(error && !goals.length) && <p>
                    {filter === 'DRAFT'
                      ? t("Vyberte si vzdálenost a termín prvního běhu.")
                      : t("Archivované cíle najdete na tomto místě.")}
                  </p>}
                  {error && !goals.length ? <button className="button secondary" onClick={() => { setError(''); setLoading(true); void load(); }}>{t('Zkusit znovu')}</button> : filter === 'DRAFT' && (
                    <button
                      className="button primary"
                      onClick={beginCreate}
                      disabled={disabled}
                    >{t("Naplánovat první běh")} </button>
                  )}
                </div>
              ) : (
                visible.map((goal) => (
                  <button
                    key={goal.id}
                    className={`panel goal-card ${selected?.id === goal.id ? 'selected' : ''}`}
                    aria-pressed={selected?.id === goal.id}
                    disabled={busy}
                    onClick={() => void detail(goal)}
                  >
                    <div className="goal-card-top">
                      <span className="goal-type"><Target size={19} aria-hidden="true" /><span className="eyebrow">{t("VENKOVNÍ BĚH")}</span></span>
                      <span className="badge">
                        {goal.state === 'DRAFT' ? t("Připravený cíl") : t("Archivováno")}
                      </span>
                    </div>
                    <strong>
                      {new Intl.NumberFormat(locale()).format(goal.target_m / 1000)} <span>km</span>
                    </strong>
                    <p><CalendarBlank size={16} aria-hidden="true" />{date(goal.starts_at)}</p>
                    <div className="goal-card-bottom">
                      <span>{t("Plánovaná jistina")} <b>{money(goal.pledge_cents, goal.currency)}</b>
                      </span>
                      <span aria-hidden="true"><ArrowRight size={20} /></span>
                    </div>
                  </button>
                ))
              )}
              {next && (
                <button className="button secondary goals-load-more" disabled={moreLoading || busy} onClick={() => void load(true)}>{moreLoading ? t('Loading…') : t("Načíst další cíle")} </button>
              )}
            </section>
            <aside className="panel goal-detail" aria-busy={detailLoading}>
              {selected ? (
                <>
                  <span className="section-icon"><Target size={25} aria-hidden="true" /></span><p className="eyebrow">
                    {selected.state === 'ARCHIVED' ? t("ARCHIVOVANÝ CÍL") : t("DETAIL CÍLE")}
                  </p>
                  <h2>{t("Uběhnout")} {selected.target_m / 1000} km</h2>
                  <p className="body-copy">{t("Jeden běh během 24 hodin, bez limitu tempa.")}</p>
                  <dl className="detail-facts">
                    <dt>{t("Začátek")}</dt>
                    <dd>{date(selected.starts_at)}</dd>
                    <dt>{t("Konec")}</dt>
                    <dd>{date(selected.ends_at)}</dd>
                    <dt>{t('Délka okna')}</dt><dd><Timer size={15} aria-hidden="true" /> {t('24 hodin')}</dd>
                  </dl>
                  {selected.state === 'DRAFT' && (
                    <div className="detail-actions">
                      <button
                        className="button primary"
                        disabled={disabled}
                        onClick={() => setEditing(selected)}
                      ><PencilSimple size={17} aria-hidden="true" />{t("Upravit cíl")} </button>
                      <button
                        className="button secondary"
                        disabled={disabled}
                        onClick={() => void archive(selected)}
                      ><Archive size={17} aria-hidden="true" />{t("Archivovat")} </button>
                    </div>
                  )}
                  <details className="section-disclosure"><summary><span>{t('Details & rules')}</span><CaretDown size={17} aria-hidden="true" /></summary><div className="disclosure-content"><dl className="detail-facts"><dt>{t('Zamýšlená jistina')}</dt><dd>{money(selected.pledge_cents, selected.currency)}</dd><dt>{t('Skutečně vloženo')}</dt><dd>{money(0, selected.currency)}</dd><dt>{t('Verze pravidel')}</dt><dd>{selected.policy_version}</dd></dl><div className="note"><strong>{t('Zatím bez finančního závazku')}</strong><p>{t('Tento cíl neuzamyká peníze. Před případným vkladem znovu potvrdíte platná pravidla, příjemce, poplatky a všechny lhůty.')}</p></div></div></details>
                  <h3 className="history-title">{t("Historie cíle")}</h3>
                  <div className="timeline">
                    {detailLoading && <p className="field-hint" role="status">{t('Loading history…')}</p>}
                    {events.map((event) => (
                      <div className="timeline-item" key={event.id}>
                        <span className="timeline-dot" />
                        <div>
                          <time>{date(event.created_at)}</time>
                          <p>
                            {event.kind === 'created'
                              ? t("Cíl vytvořen")
                              : event.kind === 'updated'
                                ? t("Parametry cíle upraveny")
                                : t("Cíl archivován")}
                          </p>
                        </div>
                      </div>
                    ))}
                  </div>
                </>
              ) : (
                <>
                  <div className="goal-intro-art" aria-hidden="true"><span /><Target size={54} weight="duotone" /><span /></div>
                  <p className="eyebrow">{t("MALÝ ZAČÁTEK. DOBRÝ POCIT.")}</p>
                  <h2>{t("Začněte cílem,")} <br />{t("který je váš.")} </h2>
                  <p className="body-copy">{t("Nemusíte běžet rychle. Vyberte si dosažitelnou vzdálenost a najděte si ve dni místo jen pro sebe.")} </p>
                  <div className="steps">
                    <p>
                      <b>01</b>{t("Vyberte 1 až 5 kilometrů.")} </p>
                    <p>
                      <b>02</b>{t("Naplánujte běžecké okno.")} </p>
                    <p>
                      <b>03</b>{t("Cíl si uložte a upravujte podle potřeby.")} </p>
                  </div>
                  <p className="field-hint">{t("Uložením cíle nic neplatíte. Vklady spustíme až s dostupným ověřováním aktivit a platební službou.")} </p>
                </>
              )}
            </aside>
          </div>
        </>
      )}
    </div>
  );
}

function localDate(iso?: string) {
  const value = iso ? new Date(iso) : new Date(Date.now() + 86400000);
  value.setMinutes(value.getMinutes() - value.getTimezoneOffset());
  return value.toISOString().slice(0, 16);
}
function GoalForm({
  initial,
  disabled,
  onSave,
  onCancel,
}: {
  initial: Goal | null;
  disabled: boolean;
  onSave: (body: GoalInput) => void;
  onCancel: () => void;
}) {
  const [target, setTarget] = useState(initial?.target_m ?? 5000);
  const currency = initial?.currency ?? 'CZK';
  const [amount, setAmount] = useState(initial?.pledge_cents ?? 10000);
  const [start, setStart] = useState(localDate(initial?.starts_at));
  const [review, setReview] = useState<GoalInput | null>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    heading.current?.focus();
  }, [review]);
  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (disabled) return;
    if (review) {
      onSave(review);
      return;
    }
    // Read the submitted field too: native date entry/autofill may not emit a React change.
    const submittedStart = new FormData(e.currentTarget).get('starts_at');
    if (typeof submittedStart !== 'string') return;
    const instant = new Date(submittedStart);
    if (Number.isNaN(instant.getTime())) return;
    setStart(submittedStart);
    setReview({ target_m: target, pledge_cents: amount, starts_at: instant.toISOString(), currency });
  }
  const reviewDate = (iso: string) =>
    new Intl.DateTimeFormat(locale(), {
      day: 'numeric',
      month: 'long',
      year: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
      timeZoneName: 'short',
    }).format(new Date(iso));
  return (
    <div className="create-layout">
      <form className="panel create-form" onSubmit={submit} aria-busy={disabled}>
        <p className="eyebrow">{initial ? t("UPRAVIT CÍL") : t("NOVÝ BĚŽECKÝ CÍL")}</p>
        <ol className="form-stepper" aria-label={t('Krok')}><li className={review ? 'complete' : 'current'} aria-current={!review ? 'step' : undefined}><span>{review ? <Check size={14} aria-hidden="true" /> : '1'}</span>{t('Plan')}</li><li className={review ? 'current' : ''} aria-current={review ? 'step' : undefined}><span>2</span>{t('Review')}</li></ol>
        <h2 ref={heading} tabIndex={-1}>
          {review ? t("Zkontrolujte svůj plán.") : t("Udělejte si čas na sebe.")}
        </h2>
        {review ? (
          <>
            <p className="body-copy">{t("Jeden venkovní běh, bez limitu tempa.")}</p>
            <dl className="detail-facts goal-review-facts">
              <dt>{t("Vzdálenost")}</dt>
              <dd>{review.target_m / 1000} km</dd>
              <dt>{t("Začátek")}</dt>
              <dd>{reviewDate(review.starts_at)}</dd>
              <dt>{t("Konec")}</dt>
              <dd>
                {reviewDate(new Date(new Date(review.starts_at).getTime() + 86400000).toISOString())}
              </dd>
              <dt>{t("Délka okna")}</dt>
              <dd>{t("24 hodin")}</dd>
              <dt>{t("Časové pásmo")}</dt>
              <dd>{Intl.DateTimeFormat().resolvedOptions().timeZone}</dd>
              <dt>{t("Zamýšlená jistina")}</dt>
              <dd>{money(review.pledge_cents, review.currency)}</dd>
              <dt>{t("Platba při uložení")}</dt>
              <dd>{money(0, review.currency)}</dd>
              <dt>{t("Viditelnost")}</dt>
              <dd>{t("Soukromý cíl, veřejně se nesdílí")}</dd>
            </dl>
            <div className="note">
              <strong>{t("Ukládáte plán bez vkladu")}</strong>
              <p>{t("Cíl můžete později upravit nebo archivovat. Nevzniká platba ani finanční závazek. Před případným vkladem samostatně potvrdíte pravidla, příjemce, poplatky a lhůty.")} </p>
            </div>
            <div className="form-actions">
              <button className="button primary" type="submit" disabled={disabled}>
                {disabled ? t("Ukládám…") : t("Uložit plán bez vkladu")}
              </button>
              <button
                className="button text-button"
                type="button"
                disabled={disabled}
                onClick={() => setReview(null)}
              >{t("Zpět k parametrům")} </button>
              <button
                className="button text-button"
                type="button"
                disabled={disabled}
                onClick={onCancel}
              >{t("Zrušit")} {initial ? t("úpravy") : t("vytváření")}
              </button>
            </div>
          </>
        ) : (
          <>
            <fieldset disabled={disabled} className="goal-distance-fieldset">
              <legend>{t("Jak daleko poběžíte?")}</legend>
              <div className="choice-row">
                {[1000, 3000, 5000].map((v) => (
                  <label key={v} className={`choice ${target === v ? 'chosen' : ''}`}>
                    <input
                      type="radio"
                      name="distance"
                      checked={target === v}
                      onChange={() => setTarget(v)}
                    />
                    <strong>
                      {v / 1000}
                      <span> km</span>
                    </strong>
                    <small>
                      {v === 1000 ? t("Malý začátek") : v === 3000 ? t("Vlastní rytmus") : t("O krok dál")}
                    </small>
                  </label>
                ))}
              </div>
            </fieldset>
            <fieldset disabled={disabled} className="goal-schedule-fieldset">
              <legend>{t('Naplánujte běžecké okno.')}</legend>
              <Field
                label={t("Kdy začnete?")}
                hint={`${t('Časové pásmo')}: ${Intl.DateTimeFormat().resolvedOptions().timeZone}. ${t('24 hodin')}.`}
              >
                <input
                  type="datetime-local"
                  name="starts_at"
                  value={start}
                  onChange={(e) => setStart(e.target.value)}
                  min={localDate(new Date(Date.now() + 6 * 60000).toISOString())}
                  max={localDate(new Date(Date.now() + 30 * 86400000).toISOString())}
                  required
                />
              </Field>
            </fieldset>
            <fieldset disabled={disabled} className="goal-pledge-fieldset">
              <legend>{t("Zamýšlená jistina")}</legend>
              <div className="amount-row">
                {(currency === 'CZK' ? [10000, 25000, 50000, 100000] : [500, 1000, 2000, 5000]).map((v) => (
                  <label className={`amount-choice ${v === amount ? 'chosen' : ''}`} key={v}>
                    <input
                      type="radio"
                      name="pledge"
                      checked={v === amount}
                      onChange={() => setAmount(v)}
                    />
                    {money(v, currency)}
                  </label>
                ))}
              </div>
              <p className="field-hint">{t("Částka se pouze uloží k vašemu plánu. Nyní ji nevkládáte.")}</p>
            </fieldset>
            <div className="form-actions">
              <button className="button primary" disabled={disabled}>{t("Zkontrolovat plán")} <ArrowRight size={18} aria-hidden="true" />
              </button>
              <button
                className="button text-button"
                type="button"
                onClick={onCancel}
                disabled={disabled}
              >{t("Zpět k cílům")} </button>
            </div>
          </>
        )}
      </form>
      <aside className="how-card">
        <div className="goal-intro-art" aria-hidden="true"><span /><Target size={54} weight="duotone" /><span /></div>
        <p className="eyebrow">{t("VÍTE, K ČEMU SE ZAVAZUJETE")}</p>
        <h3>{t("Váš plán.")} <br />{t("Žádné překvapení.")} </h3>
        <p className="body-copy">{t("Uložený cíl můžete upravit nebo archivovat. Skutečný finanční závazek vznikne až samostatným potvrzením a ověřeným vkladem, jakmile bude služba dostupná.")} </p>
        <div className="how-bottom">
          <p>{t("Teď se soustřeďte na svůj cíl.")} <br />{t("Platba se při uložení neprovádí.")} </p>
        </div>
      </aside>
    </div>
  );
}
