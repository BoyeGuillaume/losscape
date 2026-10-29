<script lang="ts">
	import { cn } from '#lib/utils.js';

	let { class: className, compact = false }: { class?: string; compact?: boolean } = $props();
</script>

<span class={cn('flex items-center gap-2 font-semibold tracking-tight', className)}>
	<span
		class="grid size-7 shrink-0 place-items-center rounded-lg bg-primary text-primary-foreground"
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
	{#if !compact}<span>losscape</span>{/if}
</span>
