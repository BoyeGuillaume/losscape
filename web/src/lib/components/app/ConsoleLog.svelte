<script lang="ts">
	import ArrowDownToLine from '@lucide/svelte/icons/arrow-down-to-line';
	import Search from '@lucide/svelte/icons/search';
	import { tick } from 'svelte';

	import { apiJson } from '#lib/api/client.js';
	import { subscribeLive, type LogLine } from '#lib/api/live.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { dateTime } from '#lib/format.js';
	import type { RunEnvironment } from '#lib/api/types.js';
	import AnsiText from './AnsiText.svelte';
	import ConsoleEmpty from './ConsoleEmpty.svelte';

	type Logs = { count: number; lines: LogLine[] };

	const ROW = 20;
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

	let count = $state(0);
	let loaded = $state(false);
	/** Scrolled to where the view opens (the end), so nothing above gets fetched first. */
	let positioned = $state(false);
	let error = $state<string | null>(null);
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

	const fetchLogs = (query: string) =>
		apiJson<Logs>(`/runs/${encodeURIComponent(run)}/logs?${query}`);

	function store(found: LogLine[], total: number) {
		for (const l of found) lines.set(l.line, l);
		count = Math.max(count, total);
		version++;
	}

	async function loadTail() {
		try {
			const tail = await fetchLogs(`tail=${PAGE}`);
			store(tail.lines, tail.count);
			loaded = true;
			if (follow) await toEnd();
			positioned = true;
		} catch (err) {
			error = String(err);
		}
	}

	function loadPage(page: number) {
		if (pages.has(page)) return;
		pages.add(page);
		fetchLogs(`from=${page * PAGE}&to=${(page + 1) * PAGE}`)
			.then((r) => store(r.lines, r.count))
			.catch(() => pages.delete(page));
	}

	$effect(() => {
		void run;
		lines = new Map();
		pages = new Set();
		count = 0;
		loaded = false;
		positioned = false;
		follow = true;
		void loadTail();
	});

	$effect(() => {
		if (!running) return;
		const id = run;
		return subscribeLive(project, [id], {
			onConsole: (c) => {
				if (c.run !== id || !c.lines.length) return;
				store(c.lines, Math.max(...c.lines.map((l) => l.line)) + 1);
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
				.catch((err) => (error = String(err)))
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
		{#if !matches && !follow && count > 0}
			<Button
				variant="outline"
				size="sm"
				class="ml-auto"
				onclick={() => ((follow = true), toEnd())}
			>
				<ArrowDownToLine />
				{running ? 'Follow' : 'Go to end'}
			</Button>
		{/if}
	</div>{/if}

{#if error}
	<p class="text-sm text-destructive">{error}</p>
{:else if matches}
	<div class="max-h-[70vh] overflow-auto rounded-xl border bg-card py-1 font-mono text-xs">
		{#each matches as l (l.line)}
			<button
				class="flex w-full min-w-max text-left leading-5 hover:bg-muted/60"
				onclick={() => jumpTo(l.line)}
				title="Show in context"
			>
				<span class="w-16 shrink-0 pr-3 text-right text-muted-foreground select-none"
					>{l.line + 1}</span
				>
				<span class="pr-4 whitespace-pre"><AnsiText text={l.text} /></span>
			</button>
		{:else}
			<p class="px-3 py-2 font-sans text-sm text-muted-foreground">No line matches.</p>
		{/each}
	</div>
{:else if loaded && count === 0}
	<ConsoleEmpty {project} {capture} {running} />
{:else}
	<div
		bind:this={viewport}
		bind:clientHeight={viewHeight}
		onscroll={onScroll}
		class="relative h-[70vh] overflow-auto rounded-xl border bg-card font-mono text-xs"
		role="log"
		aria-label="Console output"
		tabindex="-1"
	>
		<div class="relative min-w-full" style:height={`${height}px`}>
			{#each visible as i (i)}
				{@const l = lineAt(i)}
				<div
					class={[
						'absolute left-0 flex h-5 w-max min-w-full leading-5 hover:bg-muted/50',
						highlight === i && 'bg-primary/10'
					]}
					style:top={`${scrollTop + i * ROW - virtualTop}px`}
					title={l ? dateTime(l.ts_ms) : undefined}
				>
					<span
						class={[
							'sticky left-0 w-16 shrink-0 border-r bg-card pr-3 text-right text-muted-foreground select-none',
							l?.stream === 'stderr' && 'border-r-destructive/60'
						]}>{i + 1}</span
					>
					<span class="pr-4 pl-3 whitespace-pre"
						>{#if l}<AnsiText text={l.text} />{/if}</span
					>
				</div>
			{/each}
		</div>
	</div>
{/if}
