<script lang="ts">
  import type { RunSummary } from './lib/types'
  import { shortSha, timeAgo, fmtDuration } from './lib/format'

  let { entry, sha }: { entry: RunSummary, sha: string } = $props()

  // The GitHub repo lives at teranos/tsot (not tsot-roam). Hardcoded
  // so the "commit" link doesn't 404 and the "checks" link lands on
  // the aggregated CI-runs view for this sha — both workflows
  // (seer.yml + seer-browser.yml) show up in one place.
  const REPO = 'teranos/tsot'
  const commitUrl = `https://github.com/${REPO}/commit/${sha}`
  const checksUrl = `https://github.com/${REPO}/commit/${sha}/checks`

  // commit_message may include a full multi-paragraph body; the
  // subject line (first line) is what fits in the header. Anything
  // after the first newline lives in a native title tooltip so the
  // reader can hover to see more without clicking through.
  const subject = $derived(entry.commit_message.split('\n')[0] || '')
  const body = $derived(entry.commit_message)
</script>

<div class="hd">
  <div class="row">
    <a class="sha" href={commitUrl} title={sha}>{shortSha(sha)}</a>
    <span class="verdict" class:pass={entry.verdict_passed} class:fail={!entry.verdict_passed}>
      {entry.verdict_passed ? 'PASS' : 'FAIL'}
    </span>
  </div>
  {#if subject}
    <div class="subject" title={body}>{subject}</div>
  {/if}
  <div class="row meta">
    <span>{timeAgo(entry.when_unix)}</span>
    <a href={checksUrl} title="all CI runs for this commit">checks</a>
    {#if entry.ci_run_url}
      <a href={entry.ci_run_url} title="the seer-host CI run that produced this data">seer-host run</a>
    {/if}
    {#if entry.duration_secs > 0}
      <span>{fmtDuration(entry.duration_secs)}</span>
    {/if}
    {#if entry.leak_enabled}
      <span class="leak">leak</span>
    {/if}
  </div>
  {#if !entry.verdict_passed && entry.verdict_violations.length > 0}
    <ul class="violations">
      {#each entry.verdict_violations as v}
        <li>{v}</li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .hd {
    padding: 8px 12px;
    background: var(--bg-almost-black);
    border-bottom: 1px solid var(--border-on-dark);
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 0 0 auto;
  }
  .row {
    display: flex;
    gap: 10px;
    align-items: baseline;
  }
  .sha {
    color: var(--accent-on-dark);
    font-weight: 500;
    font-size: var(--font-size-md);
    text-decoration: none;
  }
  .sha:hover { text-decoration: underline; }
  .verdict {
    font-size: 10px;
    letter-spacing: 0.06em;
    padding: 1px 6px;
    border-radius: var(--border-radius);
  }
  .pass { background: var(--glyph-status-success-text); color: var(--bg-almost-black); }
  .fail { background: var(--glyph-status-error-text); color: var(--bg-almost-black); }
  .subject {
    font-size: var(--font-size-sm);
    color: var(--text-on-dark);
    line-height: 1.3;
    word-break: break-word;
  }
  .meta {
    color: var(--text-on-dark-tertiary);
    font-size: var(--font-size-xs);
  }
  .meta a {
    color: var(--accent-on-dark);
    text-decoration: none;
  }
  .meta a:hover { text-decoration: underline; }
  .leak { color: var(--color-warning); }
  .violations {
    margin: 4px 0 0 0;
    padding-left: 18px;
    color: var(--color-error);
    font-size: var(--font-size-xs);
  }
</style>
