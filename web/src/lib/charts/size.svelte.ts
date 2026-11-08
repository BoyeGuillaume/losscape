// How tall charts are. A matter of screen rather than of data, so it stays in this browser.

const KEY = 'losscape:chart-size';

export const CHART_SIZES = {
	small: { label: 'Small', height: 'h-44' },
	medium: { label: 'Medium', height: 'h-56' },
	large: { label: 'Large', height: 'h-80' }
} as const;
export type ChartSizeName = keyof typeof CHART_SIZES;

/** What was saved, if it's still a size: anything can be in storage. */
export function parseChartSize(saved: string | null): ChartSizeName {
	return saved !== null && Object.hasOwn(CHART_SIZES, saved) ? (saved as ChartSizeName) : 'medium';
}

function load(): ChartSizeName {
	try {
		return parseChartSize(localStorage.getItem(KEY));
	} catch {
		// Storage unavailable: charts keep the default size.
		return 'medium';
	}
}

class ChartSize {
	#name = $state<ChartSizeName>(load());

	get name() {
		return this.#name;
	}

	set name(next: ChartSizeName) {
		this.#name = next;
		try {
			localStorage.setItem(KEY, next);
		} catch {
			// Not kept across visits.
		}
	}

	/** Tailwind class for the height of a plot. */
	get height() {
		return CHART_SIZES[this.#name].height;
	}
}

export const chartSize = new ChartSize();
