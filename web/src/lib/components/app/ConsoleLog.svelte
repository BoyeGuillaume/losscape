<script lang="ts">
	import ArrowDownToLine from '@lucide/svelte/icons/arrow-down-to-line';
	import ArrowUpToLine from '@lucide/svelte/icons/arrow-up-to-line';
	import Download from '@lucide/svelte/icons/download';
	import Search from '@lucide/svelte/icons/search';
	import { tick } from 'svelte';

	import { stripAnsi } from '#lib/ansi.js';
	import { apiJson } from '#lib/api/client.js';
	import { subscribeLive, type LogLine } from '#lib/api/live.js';
	import { buttonVariants } from '#lib/components/ui/button/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { dateTime } from '#lib/format.js';
	import type { RunEnvironment } from '#lib/api/types.js';
	import AnsiText from './AnsiText.svelte';
	import { onRestored } from '#lib/connection.svelte.js';
	import ConsoleEmpty from './ConsoleEmpty.svelte';
	import ErrorCard from './ErrorCard.svelte';

	type Logs = { count: number; lines: LogLine[] };

	/** Row height: the font's own line height, so box drawing characters join up. */
	const ROW = 17;
	/** Lines per request; pages load as they scroll into view, the last one first. */
	const PAGE = 500;
	const OVERSCAN = 40;
	/** Browsers cap element heights; beyond this, scrolling maps onto the lines. */
	const MAX_HEIGHT = 8_000_000;
	const MAX_MATCHES = 500;

	let {
		project,
		run,
		running,
		capture
	}: {
		project: string;
		/** Hyphenated run id. */
		run: string;
		running: boolean;
		/** Whether the console was captured (from the run's environment). */
		capture: RunEnvironment['console'];
	} = $props();

	// The page reloads its run every few seconds. Through this, effects only see the id: read
	// directly, the prop would depend on the whole run and restart them on every reload.
	const runId = $derived(run);

	let count = $state(0);
	let loaded = $state(false);
	/** Scrolled to where the view opens (the end), so nothing above gets fetched first. */
	let positioned = $state(false);
	let error = $state<unknown>(null);
	/** Bumped when `lines` changes (the map itself isn't reactive: it can be large). */
	let version = $state(0);
	/* eslint-disable svelte/prefer-svelte-reactivity -- caches; `version` drives renders */
	let lines = new Map<number, LogLine>();
	/** Pages requested (loaded or loading). */
	let pages = new Set<number>();
	/* eslint-enable svelte/prefer-svelte-reactivity */

	let viewport: HTMLDivElement | undefined = $state();
	let viewHeight = $state(0);
	let scrollTop = $state(0);
	/** Stay at the end as lines arrive. */
	let follow = $state(true);
	let highlight = $state<number | null>(null);

	/** The whole log as a file (streamed by the server), plain or with its colors. */
	const downloadHref = (ansi: boolean) =>
		`/api/v0/runs/${encodeURIComponent(runId)}/logs.txt${ansi ? '?ansi=true' : ''}`;

	// Progress bars are rewritten in place, and stay where they started as the log moves on:
	// for instance tqdm's on stderr, its messages on stdout. They show pinned below the log,
	// as in a terminal, until they finish, get overwritten or stop changing.
	const BAR_STALE_MS = 600_000; // long: an outer bar moves once per epoch
	/** Other lines being rewritten (`\rstep 12`) only stay while it goes on. */
	const REWRITE_STALE_MS = 3_000;
	const MAX_LIVE = 6;
	/** Rows the strip keeps after losing some, so it doesn't resize as bars come and go. */
	const SHRINK_AFTER_MS = 3_000;

	/** Its percentage, if a line has the shape of a tqdm or rich bar. */
	function barPercent(text: string): number | null {
		const m = /(\d+)%\s*\||[━╸╺]\s*(\d+)%/.exec(stripAnsi(text));
		return m ? Number(m[1] ?? m[2]) : null;
	}

	type Live = { line: LogLine; at: number; bar: boolean; times: number };
	let live = $state.raw<Record<number, Live>>({});
	let clock = $state(Date.now());
	$effect(() => {
		if (!running) return;
		const timer = setInterval(() => (clock = Date.now()), 1000);
		return () => clearInterval(timer);
	});

	const isFresh = (r: Live, now: number) =>
		now - r.at < (r.bar ? BAR_STALE_MS : REWRITE_STALE_MS);

	/** `at`: when these changed, if not just now (lines loaded rather than pushed). */
	function noteLive(found: LogLine[], at?: (l: LogLine) => number) {
		const now = Date.now();
		const next = Object.fromEntries(
			Object.entries(live).filter(([, r]) => isFresh(r, now))
		) as typeof live;
		for (const l of found) {
			const before = next[l.line];
			const known = lines.get(l.line);
			if (known?.text === l.text) continue;
			const percent = barPercent(l.text);
			const bar = percent !== null;
			if (bar && percent < 100) {
				next[l.line] = { line: l, at: at?.(l) ?? now, bar, times: 0 };
			} else if (!bar && l.text && known && !before?.bar) {
				next[l.line] = { line: l, at: now, bar, times: (before?.times ?? 0) + 1 };
			} else {
				// Finished, cleared, or a message that took the row of a bar.
				delete next[l.line];
			}
		}
		live = next;
	}

	const fetchLogs = (query: string) =>
		apiJson<Logs>(`/runs/${encodeURIComponent(runId)}/logs?${query}`);

	/** Lines not older than what's here: requests and pushes can arrive out of order. */
	const newer = (found: LogLine[]) =>
		found.filter((l) => (lines.get(l.line)?.ts_ms ?? -Infinity) <= l.ts_ms);

	function store(found: LogLine[], total: number) {
		for (const l of found) lines.set(l.line, l);
		count = Math.max(count, total);
		version++;
	}

	async function loadTail() {
		try {
			const tail = await fetchLogs(`tail=${PAGE}`);
			const found = newer(tail.lines);
			noteLive(
				found.filter((l) => !lines.has(l.line)),
				(l) => l.ts_ms
			);
			store(found, tail.count);
			loaded = true;
			if (follow) await toEnd();
			positioned = true;
			error = null;
		} catch (err) {
			// What's here stays through a failed refresh.
			if (!loaded) error = err;
		}
	}

	function loadPage(page: number) {
		if (pages.has(page)) return;
		pages.add(page);
		fetchLogs(`from=${page * PAGE}&to=${(page + 1) * PAGE}`)
			.then((r) => store(newer(r.lines), r.count))
			.catch(() => pages.delete(page));
	}

	$effect(() => {
		void runId;
		lines = new Map();
		pages = new Set();
		live = {};
		count = 0;
		loaded = false;
		positioned = false;
		follow = true;
		void loadTail();
		return onRestored(() => void loadTail());
	});

	$effect(() => {
		if (!running) return;
		const id = runId;
		return subscribeLive(project, [id], {
			onConsole: (c) => {
				if (c.run !== id || !c.lines.length) return;
				const found = newer(c.lines);
				noteLive(found);
				store(found, Math.max(...c.lines.map((l) => l.line)) + 1);
				if (follow) void toEnd();
			},
			onReset: () => {
				pages = new Set();
				void loadTail();
			}
		});
	});

	// Virtual scrolling, scaled when the full height would be too tall for the browser.
	const virtualHeight = $derived(count * ROW);
	const height = $derived(Math.min(virtualHeight, MAX_HEIGHT));
	const scale = $derived(
		height > viewHeight ? (virtualHeight - viewHeight) / (height - viewHeight) : 1
	);
	const virtualTop = $derived(scrollTop * scale);
	const first = $derived(Math.max(0, Math.floor(virtualTop / ROW) - OVERSCAN));
	const last = $derived(Math.min(count, Math.ceil((virtualTop + viewHeight) / ROW) + OVERSCAN));
	const visible = $derived(
		Array.from({ length: Math.max(0, last - first) }, (_, i) => first + i)
	);

	// Load the pages with lines on screen that aren't here yet.
	$effect(() => {
		if (!positioned || last <= first) return;
		void version;
		for (let p = Math.floor(first / PAGE); p <= Math.floor((last - 1) / PAGE); p++) {
			const from = Math.max(first, p * PAGE);
			const to = Math.min(last, (p + 1) * PAGE);
			for (let i = from; i < to; i++) {
				if (!lines.has(i)) {
					loadPage(p);
					break;
				}
			}
		}
	});

	/** The bars and rewritten lines now, in log order. */
	const liveNow = $derived(
		running
			? Object.values(live)
					.filter((r) => (r.bar || r.times >= 2) && isFresh(r, clock))
					.map((r) => r.line)
					.sort((a, b) => a.line - b.line)
					.slice(-MAX_LIVE)
			: []
	);

	// Bars still at the very end of the log are on screen already, as in a terminal. Once the
	// log has moved past them, the strip stays until they're gone: bars can come back to the
	// end for a moment (a message moves them), and that shouldn't make it blink.
	let pinned = $state(false);
	let stripRows = $state(0);
	let lastFull = 0;
	$effect(() => {
		void clock;
		const n = liveNow.length;
		if (n >= stripRows) {
			stripRows = n;
			lastFull = Date.now();
		} else if (Date.now() - lastFull > SHRINK_AFTER_MS) {
			stripRows = n;
		}
		if (!stripRows) pinned = false;
		else if (n && liveNow[0].line < count - n) pinned = true;
	});

	const lineAt = (i: number) => {
		void version;
		return lines.get(i);
	};

	async function toEnd() {
		await tick();
		if (viewport) {
			viewport.scrollTop = viewport.scrollHeight;
			scrollTop = viewport.scrollTop; // before the scroll event, for what loads next
		}
	}

	function toTop() {
		follow = false;
		if (viewport) viewport.scrollTop = 0;
	}

	function onScroll() {
		if (!viewport) return;
		scrollTop = viewport.scrollTop;
		follow = viewport.scrollTop + viewport.clientHeight >= viewport.scrollHeight - 2 * ROW;
	}

	async function jumpTo(line: number) {
		query = '';
		matches = null; // now, so the log is back after the tick
		follow = false;
		highlight = line;
		await tick();
		if (viewport) {
			viewport.scrollTop = Math.max(0, (line * ROW - viewport.clientHeight / 2) / scale);
			scrollTop = viewport.scrollTop;
		}
		setTimeout(() => highlight === line && (highlight = null), 2500);
	}

	// Search (on the server: colors are ignored, history isn't downloaded).
	let query = $state('');
	let matches = $state<LogLine[] | null>(null);
	let searching = $state(false);
	$effect(() => {
		const q = query.trim();
		if (!q) {
			matches = null;
			return;
		}
		searching = true;
		const timer = setTimeout(() => {
			fetchLogs(new URLSearchParams({ q, limit: String(MAX_MATCHES) }).toString())
				.then((r) => q === query.trim() && (matches = r.lines))
				.catch((err) => (error = err))
				.finally(() => (searching = false));
		}, 250);
		return () => clearTimeout(timer);
	});
</script>

{#if !(loaded && count === 0)}<div class="mb-3 flex flex-wrap items-center gap-2">
		<div class="relative w-full sm:w-72">
			<Search
				class="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
			/>
			<Input
				class="pl-8"
				placeholder="Search output"
				aria-label="Search output"
				bind:value={query}
			/>
		</div>
		<span class="text-sm text-muted-foreground tabular-nums">
			{#if matches}
				{matches.length}{matches.length === MAX_MATCHES ? '+' : ''} matching lines
			{:else if searching}
				Searching…
			{:else}
				{count.toLocaleString('en')} lines
			{/if}
		</span>
		<DropdownMenu.Root>
			<DropdownMenu.Trigger
				class={buttonVariants({ variant: 'outline', size: 'sm', class: 'ml-auto' })}
				disabled={count === 0}
			>
				<Download /> Download
			</DropdownMenu.Trigger>
			<DropdownMenu.Content align="end" class="w-56">
				<!-- eslint-disable svelte/no-navigation-without-resolve -- API files, not pages -->
				<DropdownMenu.Item>
					{#snippet child({ props })}
						<a {...props} href={downloadHref(false)} download>Plain text (.log)</a>
					{/snippet}
				</DropdownMenu.Item>
				<DropdownMenu.Item>
					{#snippet child({ props })}
						<a {...props} href={downloadHref(true)} download>With colors (ANSI codes)</a
						>
					{/snippet}
				</DropdownMenu.Item>
				<!-- eslint-enable svelte/no-navigation-without-resolve -->
			</DropdownMenu.Content>
		</DropdownMenu.Root>
	</div>{/if}

{#if error}
	<ErrorCard {error} what="console output" retry={loadTail} />
{:else if matches}
	<div class="terminal max-h-[70vh] overflow-auto rounded-xl border py-2 text-[13px]">
		{#each matches as l (l.line)}
			<button
				class={[
					'block w-full min-w-max border-l-2 px-3 text-left whitespace-pre hover:bg-[var(--term-hover)]',
					l.stream === 'stderr' ? 'border-l-red-500/70' : 'border-l-transparent'
				]}
				style:line-height={`${ROW}px`}
				onclick={() => jumpTo(l.line)}
				title="Show in context"><AnsiText text={l.text} /></button
			>
		{:else}
			<p class="px-3 py-2 font-sans text-sm text-[var(--term-muted)]">No line matches.</p>
		{/each}
	</div>
{:else if loaded && count === 0}
	<ConsoleEmpty {project} {capture} {running} />
{:else}
	<div class="terminal relative overflow-hidden rounded-xl border">
		<div
			class="flex h-8 items-center gap-2 border-b border-[var(--term-border)] bg-[var(--term-bar)] px-3 font-sans text-xs text-[var(--term-muted)]"
		>
			<span class="flex gap-1.5" aria-hidden="true">
				<span class="size-2.5 rounded-full bg-[var(--term-border)]"></span>
				<span class="size-2.5 rounded-full bg-[var(--term-border)]"></span>
				<span class="size-2.5 rounded-full bg-[var(--term-border)]"></span>
			</span>
			<span class="ml-1">stdout · stderr</span>
			{#if running}
				<span class="ml-auto inline-flex items-center gap-1.5">
					<span class="size-1.5 animate-pulse rounded-full bg-emerald-500"></span>
					{follow ? 'Following' : 'Live'}
				</span>
			{/if}
		</div>
		<div class="relative">
			<div
				bind:this={viewport}
				bind:clientHeight={viewHeight}
				onscroll={onScroll}
				class="relative h-[calc(100svh-27rem)] min-h-64 overflow-auto text-[13px]"
				role="log"
				aria-label="Console output"
				tabindex="-1"
			>
				<div class="relative min-w-full" style:height={`${height + 16}px`}>
					{#each visible as i (i)}
						{@const l = lineAt(i)}
						<div
							class={[
								'absolute left-0 w-max min-w-full border-l-2 pr-4 pl-3 whitespace-pre hover:bg-[var(--term-hover)]',
								l?.stream === 'stderr'
									? 'border-l-red-500/70'
									: 'border-l-transparent',
								highlight === i && 'bg-[var(--term-mark)]'
							]}
							style:top={`${8 + scrollTop + i * ROW - virtualTop}px`}
							style:height={`${ROW}px`}
							style:line-height={`${ROW}px`}
							title={l
								? `${dateTime(l.ts_ms)}${l.stream === 'stderr' ? ' · stderr' : ''}`
								: undefined}
						>
							{#if l}<AnsiText text={l.text} />{/if}
						</div>
					{/each}
				</div>
			</div>
			<div class="absolute right-4 bottom-4 flex flex-col gap-2">
				{#if scrollTop > 0}
					<button
						class="flex items-center gap-1.5 rounded-full border border-[var(--term-border)] bg-[var(--term-button)] px-3 py-1.5 font-sans text-xs text-[var(--term-fg)] shadow-lg backdrop-blur hover:bg-[var(--term-bar)]"
						onclick={toTop}
					>
						<ArrowUpToLine class="size-3.5" /> Top
					</button>
				{/if}
				{#if !follow}
					<button
						class="flex items-center gap-1.5 rounded-full border border-[var(--term-border)] bg-[var(--term-button)] px-3 py-1.5 font-sans text-xs text-[var(--term-fg)] shadow-lg backdrop-blur hover:bg-[var(--term-bar)]"
						onclick={() => ((follow = true), toEnd())}
					>
						<ArrowDownToLine class="size-3.5" />
						{running ? 'Follow' : 'Bottom'}
					</button>
				{/if}
			</div>
		</div>
		{#if pinned && running}
			<!-- Lines stay at the bottom: newer bars keep their place as older lines leave. -->
			<div
				class="flex flex-col justify-end overflow-x-auto border-t border-[var(--term-border)] bg-[var(--term-bar)] py-1 pl-3.5 text-[13px]"
				style:min-height={`${stripRows * ROW + 8}px`}
				role="status"
				aria-label="Live progress"
				title="Lines being rewritten in place (progress bars)"
			>
				{#each liveNow as l (l.line)}
					<div class="whitespace-pre" style:line-height={`${ROW}px`}>
						<AnsiText text={l.text} />
					</div>
				{/each}
			</div>
		{/if}
	</div>
{/if}
