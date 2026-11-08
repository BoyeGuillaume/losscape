<script lang="ts">
	import CircleQuestionMark from '@lucide/svelte/icons/circle-question-mark';

	import { buttonVariants } from '#lib/components/ui/button/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
</script>

<DropdownMenu.Root>
	<DropdownMenu.Trigger
		class={buttonVariants({ variant: 'ghost', size: 'icon-sm' })}
		aria-label="Filter syntax"
	>
		<CircleQuestionMark />
	</DropdownMenu.Trigger>
	<DropdownMenu.Content align="end" class="w-96 p-3 text-xs leading-relaxed">
		<p class="mb-2 font-medium text-foreground">Filter syntax</p>
		<dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-muted-foreground">
			<dt class="font-mono text-foreground">config.lr &lt; 1e-3</dt>
			<dd>compare numbers: = != &lt; &lt;= &gt; &gt;=</dd>
			<dt class="font-mono text-foreground">summary.val/acc &gt; 0.9</dt>
			<dd>any config or summary key</dd>
			<dt class="font-mono text-foreground">tag:baseline</dt>
			<dd>also state:, alias:, group:, name:</dd>
			<dt class="font-mono text-foreground">name ~ "^bert"</dt>
			<dd>regular expression</dd>
			<dt class="font-mono text-foreground">a or (b and not c)</dt>
			<dd>combine; spaces mean and</dd>
			<dt class="font-mono text-foreground">falcon</dt>
			<dd>search names, tags, groups, aliases</dd>
		</dl>
	</DropdownMenu.Content>
</DropdownMenu.Root>
