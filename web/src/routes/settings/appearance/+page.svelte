<script lang="ts">
	import * as Card from '#lib/components/ui/card/index.js';
	import * as ToggleGroup from '#lib/components/ui/toggle-group/index.js';
	import {
		appearance,
		CHART_COLUMNS,
		CHART_SIZES,
		DEFAULTS,
		DOTS_BELOW,
		THEMES,
		type Appearance
	} from '#lib/appearance.svelte.js';

	type Key = keyof Appearance;
	type Choice = { value: string; label: string };

	const labels = <T extends Record<string, string | { label: string }>>(choices: T): Choice[] =>
		Object.entries(choices).map(([value, c]) => ({
			value,
			label: typeof c === 'string' ? c : c.label
		}));

	const SETTINGS: { key: Key; title: string; description: string; choices: Choice[] }[] = [
		{
			key: 'theme',
			title: 'Theme',
			description: 'System follows your device',
			choices: labels(THEMES)
		},
		{
			key: 'chartSize',
			title: 'Chart height',
			description: 'Of every chart',
			choices: labels(CHART_SIZES)
		},
		{
			key: 'columns',
			title: 'Charts per row',
			description: 'On a wide screen; fewer where they wouldn’t fit',
			// In this order: number-like keys come first in an object.
			choices: (['auto', '1', '2', '3', '4'] as const).map((value) => ({
				value,
				label: CHART_COLUMNS[value].label
			}))
		},
		{
			key: 'dots',
			title: 'Points as dots',
			description: `When a line has fewer than ${DOTS_BELOW} points in view`,
			choices: [
				{ value: 'true', label: 'Shown' },
				{ value: 'false', label: 'Hidden' }
			]
		}
	];

	/** Settings are strings in the controls; dots is the one boolean. */
	const toText = (v: Appearance[Key]) => String(v);
	const fromText = (key: Key, text: string) =>
		(key === 'dots' ? text === 'true' : text) as Appearance[Key];
	const labelOf = (s: (typeof SETTINGS)[number], v: Appearance[Key]) =>
		s.choices.find((c) => c.value === toText(v))?.label ?? toText(v);

	function source(key: Key): string {
		if (appearance.browser[key] !== undefined) return 'set in this browser';
		if (appearance.account[key] !== undefined) return 'from your account';
		return 'the default';
	}
</script>

<Card.Root>
	<Card.Header>
		<Card.Title>Appearance</Card.Title>
		<Card.Description>
			Your account’s settings follow you to every browser you sign in from. A browser can have
			its own, for a small laptop screen say: those win there.
		</Card.Description>
	</Card.Header>
	<Card.Content class="grid gap-6">
		{#each SETTINGS as s (s.key)}
			{@const account = appearance.account[s.key] ?? DEFAULTS[s.key]}
			{@const browser = appearance.browser[s.key]}
			<div class="grid gap-x-6 gap-y-2 sm:grid-cols-[11rem_1fr]">
				<div class="text-sm font-medium">
					{s.title}
					<p class="text-xs font-normal text-muted-foreground">{s.description}</p>
				</div>
				<div class="grid gap-2">
					<div class="flex flex-wrap items-center gap-2 text-sm">
						<span class="w-24 text-muted-foreground">Account</span>
						<ToggleGroup.Root
							type="single"
							variant="outline"
							size="sm"
							class="flex-wrap"
							value={toText(account)}
							onValueChange={(v) =>
								v && appearance.setAccount(s.key, fromText(s.key, v))}
							aria-label={`${s.title}, for your account`}
						>
							{#each s.choices as c (c.value)}
								<ToggleGroup.Item value={c.value}>{c.label}</ToggleGroup.Item>
							{/each}
						</ToggleGroup.Root>
					</div>
					<div class="flex flex-wrap items-center gap-2 text-sm">
						<span class="w-24 text-muted-foreground">This browser</span>
						<ToggleGroup.Root
							type="single"
							variant="outline"
							size="sm"
							class="flex-wrap"
							value={browser === undefined ? 'account' : toText(browser)}
							onValueChange={(v) =>
								v &&
								appearance.setBrowser(
									s.key,
									v === 'account' ? undefined : fromText(s.key, v)
								)}
							aria-label={`${s.title}, for this browser`}
						>
							<ToggleGroup.Item value="account">Same as account</ToggleGroup.Item>
							{#each s.choices as c (c.value)}
								<ToggleGroup.Item value={c.value}>{c.label}</ToggleGroup.Item>
							{/each}
						</ToggleGroup.Root>
					</div>
					<p class="text-xs text-muted-foreground">
						In use here: <span class="font-medium text-foreground"
							>{labelOf(s, appearance.current[s.key])}</span
						>, {source(s.key)}.
					</p>
				</div>
			</div>
		{/each}
	</Card.Content>
</Card.Root>
