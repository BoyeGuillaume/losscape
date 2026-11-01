<script lang="ts">
	import { css, parseAnsi } from '#lib/ansi.js';

	let { text }: { text: string } = $props();
	const segments = $derived(parseAnsi(text));
</script>

<!-- One line on purpose: whitespace between the spans would show up in the output. -->
{#each segments as s, i (i)}{@const c = css(s.style)}<span
		class={c.class}
		style:color={c.color}
		style:background-color={c.background}>{s.text}</span
	>{/each}
