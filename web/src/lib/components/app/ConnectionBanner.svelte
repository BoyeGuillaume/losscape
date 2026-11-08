<script lang="ts">
	import DatabaseZap from '@lucide/svelte/icons/database-zap';
	import WifiOff from '@lucide/svelte/icons/wifi-off';

	import { Button } from '#lib/components/ui/button/index.js';
	import { connection, retryNow } from '#lib/connection.svelte.js';

	const TITLES = {
		offline: "You're offline",
		server: "Can't reach the server",
		storage: "The server can't reach its database"
	};
</script>

<!-- Over the page rather than in it: what's on screen stays put. The header is free here. -->
{#if connection.problem}
	<div
		role="status"
		class="fixed top-2 right-4 z-50 flex w-max max-w-[calc(100vw-2rem)] items-center gap-2 rounded-lg border bg-popover py-1 pr-1 pl-3 text-sm text-popover-foreground shadow-md"
	>
		{#if connection.problem === 'storage'}
			<DatabaseZap class="size-4 shrink-0 text-destructive" />
		{:else}
			<WifiOff class="size-4 shrink-0 text-destructive" />
		{/if}
		<span class="font-medium">{TITLES[connection.problem]}</span>
		<span class="hidden text-muted-foreground md:inline">
			Showing what's loaded. Retrying…
		</span>
		<Button variant="ghost" size="sm" class="h-7 px-2" onclick={retryNow}>Retry now</Button>
	</div>
{/if}
