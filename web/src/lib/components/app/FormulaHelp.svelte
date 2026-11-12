<script lang="ts">
	import CircleQuestionMark from '@lucide/svelte/icons/circle-question-mark';

	import { buttonVariants } from '#lib/components/ui/button/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import { AGGREGATE_NAMES, CONSTANT_NAMES, FUNCTION_NAMES } from '#lib/metrics/formula.js';
</script>

<DropdownMenu.Root>
	<DropdownMenu.Trigger
		class={buttonVariants({ variant: 'ghost', size: 'icon-xs' })}
		aria-label="Formula syntax"
		title="Formula syntax"
	>
		<CircleQuestionMark />
	</DropdownMenu.Trigger>
	<DropdownMenu.Content align="start" class="w-[30rem] p-3 text-xs leading-relaxed">
		<p class="mb-2 font-medium text-foreground">Formula syntax</p>
		<dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-muted-foreground">
			<dt class="font-mono text-foreground">[val/loss]</dt>
			<dd>a metric, at each step: its name in brackets</dd>
			<dt class="font-mono text-foreground">[a] * 2 + [b] ^ 0.5</dt>
			<dd>+ - * / and ^ (power), with parentheses</dd>
			<dt class="font-mono text-foreground">max([a], [b])</dt>
			<dd>functions, at each step: abs, clamp, round, log…</dd>
			<dt class="font-mono text-foreground">pow([x], 2), sqrt([x])</dt>
			<dd>powers and roots</dd>
			<dt class="font-mono text-foreground">sin([x]), atan2([y], [x])</dt>
			<dd>trigonometry, in radians</dd>
			<dt class="font-mono text-foreground">mean([val/loss])</dt>
			<dd>over the whole run: lowest, highest, mean, median</dd>
			<dt class="font-mono text-foreground">[loss] / highest([loss])</dt>
			<dd>each step compared with the whole run</dd>
			<dt class="font-mono text-foreground">2 * pi, e</dt>
			<dd>constants</dd>
		</dl>
		<p class="mt-2 text-muted-foreground">
			A metric logged once, like <span class="font-mono">model/params</span>, counts at every
			step. Mean and median show on charts; tables leave them empty. Type a name in the
			formula to see what it does.
		</p>
		<p class="mt-2 text-muted-foreground">
			<span class="text-foreground">All of them:</span>
			<span class="font-mono"
				>{[...FUNCTION_NAMES, ...AGGREGATE_NAMES, ...CONSTANT_NAMES].join(', ')}</span
			>
		</p>
	</DropdownMenu.Content>
</DropdownMenu.Root>
