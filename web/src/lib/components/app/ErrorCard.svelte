<script lang="ts">
	import type { Component, Snippet } from 'svelte';
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import DatabaseZap from '@lucide/svelte/icons/database-zap';
	import Lock from '@lucide/svelte/icons/lock';
	import SearchX from '@lucide/svelte/icons/search-x';
	import WifiOff from '@lucide/svelte/icons/wifi-off';

	import { Button } from '#lib/components/ui/button/index.js';
	import { describeError, type ErrorKind } from '#lib/errors.js';

	import EmptyCard from './EmptyCard.svelte';

	/** In place of something that couldn't be loaded: what happened, and what can be done. */
	let {
		error,
		what,
		retry,
		children
	}: {
		error: unknown;
		/** What was being loaded: "run", "project list". */
		what: string;
		retry?: () => void;
		/** More ways out, such as a link back. */
		children?: Snippet;
	} = $props();

	const ICONS: Record<ErrorKind, Component> = {
		connection: WifiOff,
		storage: DatabaseZap,
		missing: SearchX,
		denied: Lock,
		failed: CircleAlert
	};

	const described = $derived(describeError(error, what));
	const canRetry = $derived(retry && described.kind !== 'missing' && described.kind !== 'denied');
</script>

<div role="alert">
	<EmptyCard icon={ICONS[described.kind]} title={described.title}>
		{#snippet description()}{described.message}{/snippet}
		{#if canRetry || children}
			<div class="flex flex-wrap gap-2">
				{#if canRetry}
					<Button variant="outline" size="sm" onclick={() => retry?.()}>Try again</Button>
				{/if}
				{@render children?.()}
			</div>
		{/if}
	</EmptyCard>
</div>
