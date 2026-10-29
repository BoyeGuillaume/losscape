<script lang="ts" module>
	/** Backgrounds dark enough for white initials (>= 4.5:1 contrast). */
	const BACKGROUNDS = [
		'bg-blue-600',
		'bg-violet-600',
		'bg-pink-600',
		'bg-red-600',
		'bg-orange-700',
		'bg-amber-700',
		'bg-green-700',
		'bg-teal-700',
		'bg-cyan-700',
		'bg-slate-600'
	];

	/** FNV-1a: the same name always gets the same color. */
	export function avatarBackground(name: string): string {
		let hash = 2166136261;
		for (const char of name.toLowerCase()) {
			hash ^= char.codePointAt(0)!;
			hash = Math.imul(hash, 16777619);
		}
		return BACKGROUNDS[(hash >>> 0) % BACKGROUNDS.length];
	}

	/** "Ada Lovelace" -> "AL", "ada.l" -> "AL", "ada" -> "A". */
	export function initials(name: string): string {
		const parts = name.split(/[\s._-]+/).filter(Boolean);
		return (
			parts
				.slice(0, 2)
				.map((p) => [...p][0].toUpperCase())
				.join('') || '?'
		);
	}
</script>

<script lang="ts">
	import { cn } from '#lib/utils.js';

	let {
		name,
		src = null,
		class: className
	}: { name: string; src?: string | null; class?: string } = $props();
</script>

{#if src}
	<img
		{src}
		alt=""
		class={cn('size-8 shrink-0 rounded-full bg-muted object-cover', className)}
		draggable="false"
	/>
{:else}
	<span
		class={cn(
			'inline-grid size-8 shrink-0 place-items-center rounded-full text-xs font-semibold text-white select-none',
			avatarBackground(name),
			className
		)}
		aria-hidden="true">{initials(name)}</span
	>
{/if}
