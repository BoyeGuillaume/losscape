<script lang="ts">
	import Check from '@lucide/svelte/icons/check';
	import Search from '@lucide/svelte/icons/search';
	import Tag from '@lucide/svelte/icons/tag';
	import { resolve } from '$app/paths';

	import { apiSend } from '#lib/api/client.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { literal } from '#lib/filter-suggest.js';
	import { withToast } from '#lib/notify.js';
	import { tagPath } from '#lib/tags/actions.js';
	import { SWATCH_CLASSES, TAG_COLORS, colorFromName, type TagColor } from '#lib/tags/palette.js';
	import { projectTags, useProjectTags } from '#lib/tags/store.svelte.js';
	import EmptyCard from './EmptyCard.svelte';
	import TagChip from './TagChip.svelte';

	let { project, canEdit }: { project: string; canEdit: boolean } = $props();

	useProjectTags(() => project);
	const tags = $derived(projectTags.project === project ? projectTags.list : []);
	let query = $state('');
	const shown = $derived(
		tags.filter((t) => t.name.toLowerCase().includes(query.trim().toLowerCase()))
	);
	/** Descriptions being typed, by tag. */
	let drafts = $state<Record<string, string>>({});

	const runsHref = (name: string) =>
		`${resolve('/p/[project]/(tabs)/runs', { project })}?q=${encodeURIComponent(`tag:${literal(name)}`)}`;

	async function save(name: string, color: string | null, description: string) {
		await withToast(() => apiSend('PUT', tagPath(project, name), { color, description }), {
			done: `${name} saved`,
			detail:
				color === null
					? 'Its color comes from its name.'
					: `It shows in ${color} everywhere in ${project}.`,
			failed: `Couldn't save ${name}`
		});
		projectTags.refresh();
	}

	function setColor(name: string, color: TagColor | null) {
		void save(name, color, projectTags.info(name)?.description ?? '');
	}

	function saveDescription(name: string) {
		const draft = drafts[name];
		const info = projectTags.info(name);
		if (draft === undefined || draft.trim() === (info?.description ?? '')) return;
		void save(name, info?.color ?? null, draft.trim());
	}
</script>

{#if tags.length === 0}
	<EmptyCard icon={Tag} title="No tags yet">
		{#snippet description()}
			Tag runs on the Runs tab (tick them, then Tags), on a run's page, or from the SDK with
			<code class="font-mono text-xs">lsc.init(tags=["baseline"])</code>. Their colors and
			descriptions are set here.
		{/snippet}
	</EmptyCard>
{:else}
	<Card.Root>
		<Card.Header>
			<Card.Title>Tags</Card.Title>
			<Card.Description>
				Each tag's color (or Auto: one that follows from its name) and a description, shown
				when hovering it. {#if !canEdit}Only editors can change them.{/if}
			</Card.Description>
		</Card.Header>
		<Card.Content class="grid gap-3">
			<div class="relative w-full sm:w-72">
				<Search
					class="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
				/>
				<Input
					class="h-8 pl-8"
					placeholder="Find a tag"
					aria-label="Find a tag"
					bind:value={query}
				/>
			</div>
			<ul class="divide-y rounded-lg border">
				{#each shown as tag (tag.name)}
					{@const current = tag.color}
					<li
						class="grid gap-2 px-3 py-3 lg:grid-cols-[minmax(10rem,14rem)_auto_1fr] lg:items-center lg:gap-4"
					>
						<div class="flex min-w-0 items-center gap-2">
							<TagChip name={tag.name} />
							<!-- eslint-disable svelte/no-navigation-without-resolve -- resolved, plus a query -->
							<a
								class="shrink-0 text-xs text-muted-foreground tabular-nums hover:text-foreground hover:underline"
								href={runsHref(tag.name)}
								title={`Runs tagged ${tag.name}`}
								>{tag.runs.toLocaleString('en')}
								{tag.runs === 1 ? 'run' : 'runs'}</a
							>
							<!-- eslint-enable svelte/no-navigation-without-resolve -->
						</div>
						<div
							class="flex flex-wrap items-center gap-1"
							role="radiogroup"
							aria-label={`Color of ${tag.name}`}
						>
							<button
								role="radio"
								aria-checked={current === null}
								disabled={!canEdit}
								class={[
									'h-5 rounded-full border px-2 text-[11px] text-muted-foreground',
									current === null && 'border-foreground text-foreground'
								]}
								title={`From the name (${colorFromName(tag.name)})`}
								onclick={() => setColor(tag.name, null)}>Auto</button
							>
							{#each TAG_COLORS as color (color)}
								<button
									role="radio"
									aria-checked={current === color}
									aria-label={color}
									title={color}
									disabled={!canEdit}
									class={[
										'flex size-5 items-center justify-center rounded-full text-white ring-offset-2 ring-offset-background',
										SWATCH_CLASSES[color],
										current === color && 'ring-2 ring-foreground'
									]}
									onclick={() => setColor(tag.name, color)}
								>
									{#if current === color}<Check
											class="size-3"
											strokeWidth={3}
										/>{/if}
								</button>
							{/each}
						</div>
						<Input
							class="h-8"
							placeholder="Description (optional)"
							aria-label={`Description of ${tag.name}`}
							disabled={!canEdit}
							maxlength={500}
							value={drafts[tag.name] ?? tag.description}
							oninput={(e) => (drafts[tag.name] = e.currentTarget.value)}
							onblur={() => saveDescription(tag.name)}
							onkeydown={(e) => e.key === 'Enter' && saveDescription(tag.name)}
						/>
					</li>
				{:else}
					<li class="px-3 py-3 text-sm text-muted-foreground">No tag matches.</li>
				{/each}
			</ul>
		</Card.Content>
	</Card.Root>
{/if}
