<script lang="ts">
	import Check from '@lucide/svelte/icons/check';
	import Copy from '@lucide/svelte/icons/copy';
	import FileCode from '@lucide/svelte/icons/file-code';
	import SquareTerminal from '@lucide/svelte/icons/square-terminal';

	import { markMatches, tokenize, type Lang, type Token } from '#lib/highlight.js';

	let {
		code,
		lang,
		title,
		highlight = []
	}: {
		code: string;
		lang: Lang;
		/** Shown in the header: a file name, or "Terminal". Defaults to the language. */
		title?: string;
		/** Text to mark wherever it is, ignoring case (search matches). */
		highlight?: string[];
	} = $props();

	// Plain text until the highlighter has loaded (it loads on first use).
	let lines = $state<Token[][] | null>(null);
	$effect(() => {
		const source = code;
		let current = true;
		tokenize(source, lang).then((t) => current && (lines = t));
		return () => (current = false);
	});
	const plain = $derived(code.split('\n').map((line) => [{ content: line } as Token]));
	/** Lines with their newline as a last token (whitespace inside <pre> is shown). */
	const shown = $derived(
		markMatches(lines ?? plain, highlight).map((line, i, all) =>
			i < all.length - 1 ? [...line, { content: '\n' }] : line
		)
	);

	let copied = $state(false);
	async function copy() {
		await navigator.clipboard?.writeText(code);
		copied = true;
		setTimeout(() => (copied = false), 1500);
	}
</script>

<figure class="code-block overflow-hidden rounded-lg border bg-muted/40 text-sm">
	<figcaption
		class="flex h-9 items-center gap-2 border-b bg-muted/60 px-3 text-xs text-muted-foreground"
	>
		{#if lang === 'shellscript'}<SquareTerminal class="size-3.5" />{:else}<FileCode
				class="size-3.5"
			/>{/if}
		<span class="truncate"
			>{title ?? { python: 'Python', shellscript: 'Terminal', yaml: 'YAML' }[lang]}</span
		>
		<button
			class="-mr-1 ml-auto rounded-md p-1 hover:bg-background hover:text-foreground"
			onclick={copy}
			aria-label="Copy code"
			title="Copy"
		>
			{#if copied}<Check class="size-3.5" />{:else}<Copy class="size-3.5" />{/if}
		</button>
	</figcaption>
	<!-- One line on purpose: whitespace inside <pre> is shown. -->
	<pre class="overflow-x-auto p-3.5 font-mono text-[13px] leading-6"><code
			>{#each shown as line, i (i)}{#each line as t, j (j)}{#if t.match}<mark
							class="rounded-sm bg-yellow-300/60 dark:bg-yellow-500/35"
							><span style:--light={t.light} style:--dark={t.dark}>{t.content}</span
							></mark
						>{:else}<span style:--light={t.light} style:--dark={t.dark}
							>{t.content}</span
						>{/if}{/each}{/each}</code
		></pre>
</figure>
