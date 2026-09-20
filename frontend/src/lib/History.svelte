<script lang="ts">
  import { untrack } from 'svelte';
  import { SvelteURLSearchParams } from 'svelte/reactivity';
  import { api, type User } from './api';
  import { time } from './playback';
  let {
    user,
    audit = false,
    statistics = false,
  } = $props<{ user: User; audit?: boolean; statistics?: boolean }>();
  type Row = {
    id: number;
    title?: string;
    username?: string;
    device?: string;
    started_at?: number;
    created_at?: number;
    played_seconds?: number;
    position?: number;
    duration?: number;
    edition?: string;
    state?: string;
    action?: string;
    target?: string;
    actor?: string;
  };
  type Result = {
    items: Row[];
    daily?: { date: string; plays: number; played_seconds: number }[];
    top?: { title: string; plays: number; played_seconds: number }[];
    next_before: number | null;
    stats?: {
      plays: number;
      played_seconds: number;
      media_count: number;
      user_count: number;
    };
    users?: {
      id: string;
      username: string;
      plays: number;
      played_seconds: number;
    }[];
  };
  let result = $state<Result | null>(null),
    people = $state<User[]>([]),
    all = $state(false),
    domain = $state('library'),
    person = $state(''),
    since = $state(''),
    until = $state(''),
    error = $state(''),
    busy = $state(false);
  $effect(() => {
    const mode = audit;
    untrack(() => {
      void load(mode);
      if (user.role === 'admin')
        void api<{ items: User[] }>('/users')
          .then((v) => (people = v.items))
          .catch((e) => (error = String(e)));
    });
  });
  async function load(mode = audit, more = false) {
    busy = true;
    error = '';
    try {
      const query = new SvelteURLSearchParams();
      if (!mode) query.set('domain', domain);
      if (more && result?.next_before)
        query.set('before', String(result.next_before));
      if (since)
        query.set('since', String(Date.parse(`${since}T00:00:00Z`) / 1000));
      if (until)
        query.set(
          'until',
          String(Date.parse(`${until}T00:00:00Z`) / 1000 + 86400),
        );
      if (all && person) query.set('user', person);
      const response = await api<Result>(
        `${mode ? '/admin/audit' : all ? '/admin/history' : '/me/history'}?${query}`,
      );
      result = more
        ? { ...response, items: [...(result?.items ?? []), ...response.items] }
        : response;
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  function date(value?: number) {
    return value === undefined
      ? ''
      : new Intl.DateTimeFormat('en', {
          dateStyle: 'medium',
          timeStyle: 'short',
          timeZone: user.timezone,
        }).format(value * 1000);
  }
</script>

{#if error}<p role="alert" class="error">{error}</p>{/if}
<section class="panel">
  <div class="section-heading">
    <h2>
      {audit
        ? 'Administrative activity'
        : statistics
          ? 'Watching statistics'
          : 'Playback history'}
    </h2>
    <button class="secondary" disabled={busy} onclick={() => void load()}
      >Refresh</button
    >
  </div>
  <p class="muted">Times shown in {user.timezone}.</p>
  {#if !audit}<form
      class="inline-form"
      onsubmit={(e) => {
        e.preventDefault();
        void load();
      }}
    >
      <label
        >Media<select bind:value={domain} onchange={() => void load()}
          ><option value="library">Local library</option><option value="youtube"
            >YouTube</option
          ><option value="twitch">Twitch</option><option value="kick"
            >Kick</option
          ></select
        ></label
      >
      {#if user.role === 'admin'}<label
          >History scope<select
            aria-label="History scope"
            bind:value={all}
            onchange={(event) => {
              all = event.currentTarget.value === 'true';
              person = '';
              void load();
            }}
            ><option value={false}>My history</option><option value={true}
              >All users</option
            ></select
          ></label
        >{#if all}<label
            >User<select bind:value={person}
              ><option value="">Everyone</option
              >{#each people as p (p.id)}<option value={p.id}
                  >{p.username}</option
                >{/each}</select
            ></label
          >{/if}{/if}
      <label>From date (UTC)<input type="date" bind:value={since} /></label
      ><label>Through date (UTC)<input type="date" bind:value={until} /></label
      ><button class="primary" disabled={busy}>Filter history</button>
    </form>{/if}
  {#if result?.stats}<div class="stats">
      <div>
        <strong>{time(result.stats.played_seconds)}</strong><small
          >Time watched</small
        >
      </div>
      <div>
        <strong>{result.stats.plays.toLocaleString()}</strong><small
          >Plays</small
        >
      </div>
      <div>
        <strong>{result.stats.media_count.toLocaleString()}</strong><small
          >Unique titles</small
        >
      </div>
      <div>
        <strong
          >{time(
            result.stats.plays
              ? result.stats.played_seconds / result.stats.plays
              : 0,
          )}</strong
        ><small>Average per play</small>
      </div>
    </div>
    <p class="muted">Playback time excludes seeks.</p>{/if}
  {#if statistics}
    <section class="chart-panel" aria-label="Daily watch time">
      <h3>Watch time by day</h3>
      <p class="muted">Most recent 90 active days, grouped in UTC.</p>
      {#if result?.daily?.length}<div class="watch-chart">
          {#each [...result.daily].reverse() as day (day.date)}<div
              class="chart-column"
              title={day.date +
                ': ' +
                time(day.played_seconds) +
                ' · ' +
                day.plays +
                ' plays'}
            >
              <div
                class="chart-bar"
                style:height={Math.max(
                  1,
                  (day.played_seconds /
                    Math.max(1, ...result.daily.map((d) => d.played_seconds))) *
                    100,
                ) + '%'}
              ></div>
              <span>{day.date.slice(5)}</span>
            </div>{/each}
        </div>
        <details>
          <summary>Daily figures</summary>
          <table>
            <thead
              ><tr><th>Date (UTC)</th><th>Plays</th><th>Time watched</th></tr
              ></thead
            ><tbody
              >{#each result.daily as day (day.date)}<tr
                  ><td>{day.date}</td><td>{day.plays}</td><td
                    >{time(day.played_seconds)}</td
                  ></tr
                >{/each}</tbody
            >
          </table>
        </details>
      {:else}<p class="empty-chart">
          Play something to start your watching statistics.
        </p>{/if}
    </section>
    <section class="chart-panel">
      <h3>Most watched titles</h3>
      {#each result?.top ?? [] as item, index (index)}<div class="rank-row">
          <span class="rank">{String(index + 1).padStart(2, '0')}</span>
          <div>
            <strong>{item.title}</strong><small>{item.plays} plays</small>
            <div
              class="rank-bar"
              style:width={Math.max(
                1,
                (item.played_seconds /
                  Math.max(
                    1,
                    ...(result?.top ?? []).map((t) => t.played_seconds),
                  )) *
                  100,
              ) + '%'}
            ></div>
          </div>
          <span>{time(item.played_seconds)}</span>
        </div>{:else}<p class="muted">No activity in this view yet.</p>{/each}
    </section>
  {/if}
  {#if all && result?.users?.length}<h3>By user</h3>
    {#each result.users as p (p.id)}<div class="row">
        <span>{p.username}</span><small
          >{p.plays} plays · {time(p.played_seconds)} played</small
        >
      </div>{/each}{/if}
  {#if !statistics}
    {#each result?.items ?? [] as row (row.id)}<div class="row">
        <div>
          <strong>{audit ? row.action : row.title}</strong><small
            >{date(audit ? row.created_at : row.started_at)}{audit
              ? ` · ${row.actor ?? 'Deleted user'}`
              : all
                ? ` · ${row.username}`
                : ''}</small
          ><small
            >{audit
              ? row.target
              : `${row.device} · ${row.edition || 'Original edition'}`}</small
          >
        </div>
        {#if !audit}<span
            >{time(row.position ?? 0)} / {time(row.duration ?? 0)}</span
          ><small>{time(row.played_seconds ?? 0)} played · {row.state}</small
          >{/if}
      </div>{:else}<p class="muted">No activity in this view yet.</p>{/each}
    {#if result?.next_before}<button
        class="secondary"
        disabled={busy}
        onclick={() => void load(audit, true)}>Load older activity</button
      >{/if}
  {/if}
</section>

<style>
  .chart-panel {
    margin-top: 28px;
    border-top: 1px solid var(--line);
    padding-top: 24px;
  }
  .chart-panel h3 {
    margin-bottom: 8px;
  }
  .chart-panel p {
    font-size: 11px;
  }
  .watch-chart {
    display: flex;
    gap: 8px;
    align-items: stretch;
    height: 210px;
    padding: 15px 0 30px;
    overflow-x: auto;
  }
  .chart-column {
    flex: 1;
    min-width: 24px;
    max-width: 80px;
    display: flex;
    justify-content: flex-end;
    flex-direction: column;
    position: relative;
  }
  .chart-bar {
    background: var(--accent-soft);
    border-top: 2px solid var(--accent);
    min-height: 2px;
    transition: background 0.2s;
  }
  .chart-column:hover .chart-bar {
    background: color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .chart-column span {
    position: absolute;
    bottom: -21px;
    font:
      9px ui-monospace,
      monospace;
    color: var(--muted);
  }
  .empty-chart {
    display: grid;
    place-items: center;
    height: 170px;
    border: 1px dashed var(--line);
  }
  .rank-row {
    display: flex;
    align-items: center;
    gap: 20px;
    padding: 18px 0;
    border-bottom: 1px solid var(--line);
    font-size: 12px;
  }
  .rank-row > div {
    flex: 1;
  }
  .rank {
    color: var(--muted);
    font:
      11px ui-monospace,
      monospace;
  }
  .rank-bar {
    height: 2px;
    background: var(--accent);
    margin-top: 10px;
  }
  .inline-form {
    margin: 16px 0;
  }
</style>
