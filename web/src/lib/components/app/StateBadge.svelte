<script lang="ts">
	import type { RunState } from '#lib/api/types.js';
	import { cn } from '#lib/utils.js';

	let { state, class: className }: { state: RunState; class?: string } = $props();

	const tone: Record<RunState, string> = {
		running: 'text-state-running bg-state-running/10 border-state-running/25',
		finished: 'text-state-finished bg-state-finished/10 border-state-finished/25',
		failed: 'text-state-failed bg-state-failed/10 border-state-failed/25',
		crashed: 'text-state-crashed bg-state-crashed/10 border-state-crashed/25',
		killed: 'text-state-killed bg-state-killed/10 border-state-killed/25'
	};
</script>

<!-- Color is never alone: the state name is always written. -->
<span
	class={cn(
		'inline-flex h-5 items-center gap-1.5 rounded-full border px-2 text-xs font-medium capitalize',
		tone[state],
		className
	)}
>
	<span class={cn('size-1.5 rounded-full bg-current', state === 'running' && 'animate-pulse')}
	></span>
	{state}
</span>
