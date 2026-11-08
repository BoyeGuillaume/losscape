<script lang="ts">
	import SearchX from '@lucide/svelte/icons/search-x';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';

	import EmptyCard from '#lib/components/app/EmptyCard.svelte';
	import ErrorCard from '#lib/components/app/ErrorCard.svelte';
	import { Button, buttonVariants } from '#lib/components/ui/button/index.js';
	import { ApiError } from '#lib/api/client.js';
	import { connection, onRestored } from '#lib/connection.svelte.js';

	const missing = $derived(page.status === 404);
	const failure = $derived(
		connection.problem
			? new ApiError(0, 'unreachable', "The server can't be reached.")
			: page.error
	);

	// A page that couldn't load for lack of a connection loads when there's one again. Only
	// then: a page that failed by itself would fail again, and reload forever.
	let wasLost = false;
	$effect(() => {
		if (connection.problem) wasLost = true;
	});
	$effect(() => {
		if (!missing) return onRestored(() => wasLost && location.reload());
	});
</script>

<div class="mx-auto max-w-[1600px]">
	{#if missing}
		<EmptyCard icon={SearchX} title="Page not found">
			{#snippet description()}
				There's nothing at
				<code class="font-mono text-xs break-all">{page.url.pathname}</code>. The link may
				be out of date, or the address mistyped.
			{/snippet}
			<div class="flex flex-wrap gap-2">
				<a class={buttonVariants({ size: 'sm' })} href={resolve('/')}>Go to projects</a>
				<Button variant="outline" size="sm" onclick={() => history.back()}>Go back</Button>
			</div>
		</EmptyCard>
	{:else}
		<ErrorCard error={failure} what="page" retry={() => location.reload()} />
	{/if}
</div>
