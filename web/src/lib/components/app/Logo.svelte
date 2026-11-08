<script lang="ts">
	import { app } from '#lib/app.svelte.js';
	import { cn } from '#lib/utils.js';

	let {
		class: className,
		markClass,
		compact = false
	}: { class?: string; markClass?: string; compact?: boolean } = $props();
</script>

<span class={cn('flex items-center gap-2 font-semibold tracking-tight', className)}>
	{#if app.info?.favicon}
		<img
			class={cn('size-7 shrink-0 rounded-lg object-contain', markClass)}
			src={app.info.favicon}
			alt=""
		/>
	{:else}
		<span
			class={cn(
				'grid size-7 shrink-0 place-items-center rounded-lg bg-primary text-primary-foreground',
				markClass
			)}
			aria-hidden="true"
		>
			<svg
				viewBox="0 0 24 24"
				class="size-4"
				fill="none"
				stroke="currentColor"
				stroke-width="2.25"
			>
				<path
					d="M3 17c3 0 4-10 7-10s4 7 7 7 2-3 4-3"
					stroke-linecap="round"
					stroke-linejoin="round"
				/>
			</svg>
		</span>
	{/if}
	{#if !compact}<span>{app.name}</span>{/if}
</span>
