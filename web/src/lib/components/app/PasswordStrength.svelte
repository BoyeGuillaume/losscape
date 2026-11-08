<script lang="ts">
	import type { passwordStrength } from '#lib/password.js';

	let {
		password,
		personal = []
	}: {
		password: string;
		/** What's known about its owner: username, display name. */
		personal?: string[];
	} = $props();

	// Loaded when first needed: its letter statistics and words are all most pages don't use.
	let rate = $state.raw<typeof passwordStrength | null>(null);
	$effect(() => {
		void import('#lib/password.js').then((module) => (rate = module.passwordStrength));
	});
	const strength = $derived(
		rate?.(password, personal) ?? { bits: 0, level: 0, label: '', tips: [] }
	);
	const bits = $derived(Math.round(strength.bits));
	/** Full at 100 bits, with a sliver from the first character. */
	const fill = $derived(password ? Math.max(4, Math.min(100, strength.bits)) : 0);
	const tone = $derived(
		[
			'bg-red-600 dark:bg-red-500',
			'bg-orange-500',
			'bg-amber-500',
			'bg-emerald-600 dark:bg-emerald-500',
			'bg-emerald-700 dark:bg-emerald-400'
		][strength.level]
	);
</script>

<div class="grid gap-1.5" data-testid="password-strength">
	<div
		class="h-1.5 overflow-hidden rounded-full bg-muted"
		role="meter"
		aria-label="Password strength"
		aria-valuemin={0}
		aria-valuemax={100}
		aria-valuenow={Math.round(fill)}
		aria-valuetext={password ? `${strength.label}, about ${bits} bits` : 'Empty'}
	>
		<div class={['h-full rounded-full transition-all', tone]} style:width={`${fill}%`}></div>
	</div>
	<div class="grid gap-1 text-xs text-muted-foreground" aria-live="polite">
		{#if password}
			<p>
				<span class="font-medium text-foreground">{strength.label}</span>
				<span
					class="tabular-nums"
					title="An estimate of how hard it is to guess: each extra bit doubles it"
					>· about {bits} {bits === 1 ? 'bit' : 'bits'}</span
				>
			</p>
			{#if strength.tips.length}
				<ul class="grid list-disc gap-0.5 pl-4">
					{#each strength.tips as tip (tip)}<li>{tip}</li>{/each}
				</ul>
			{/if}
		{:else}
			<p>Any password works. Longer is stronger: several unrelated words work well.</p>
		{/if}
	</div>
</div>
