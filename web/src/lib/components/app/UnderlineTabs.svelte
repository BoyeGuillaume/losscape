<script lang="ts">
	/** The tab bar of project and run pages: links to pages, or tabs of one page. */
	let {
		label,
		items,
		current,
		onSelect,
		class: className
	}: {
		label: string;
		/** With `href`: links; without: tabs (`onSelect`, panels `id="panel-<id>"`). */
		items: { id: string; label: string; href?: string }[];
		current: string;
		onSelect?: (id: string) => void;
		class?: string;
	} = $props();

	const linkClass = (active: boolean) => [
		'-mb-px border-b-2 px-3 pb-2 text-sm font-medium transition-colors',
		active
			? 'border-foreground text-foreground'
			: 'border-transparent text-muted-foreground hover:text-foreground'
	];

	/** Arrow keys move between tabs. */
	function keydown(event: KeyboardEvent) {
		const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
		if (!step || !onSelect) return;
		const at = items.findIndex((i) => i.id === current);
		const next = items[(at + step + items.length) % items.length];
		onSelect(next.id);
		(event.currentTarget as HTMLElement)
			.querySelector<HTMLElement>(`[data-tab="${next.id}"]`)
			?.focus();
	}
</script>

{#if onSelect}
	<div
		class={['flex gap-1 overflow-x-auto border-b', className]}
		role="tablist"
		aria-label={label}
		tabindex="-1"
		onkeydown={keydown}
	>
		{#each items as item (item.id)}
			<button
				role="tab"
				id={`tab-${item.id}`}
				data-tab={item.id}
				aria-selected={current === item.id}
				aria-controls={`panel-${item.id}`}
				tabindex={current === item.id ? 0 : -1}
				class={linkClass(current === item.id)}
				onclick={() => onSelect(item.id)}>{item.label}</button
			>
		{/each}
	</div>
{:else}
	<!-- eslint-disable svelte/no-navigation-without-resolve -- hrefs are resolved by the caller -->
	<nav class={['flex gap-1 overflow-x-auto border-b', className]} aria-label={label}>
		{#each items as item (item.id)}
			<a
				href={item.href}
				aria-current={current === item.id ? 'page' : undefined}
				class={linkClass(current === item.id)}>{item.label}</a
			>
		{/each}
	</nav>
	<!-- eslint-enable svelte/no-navigation-without-resolve -->
{/if}
