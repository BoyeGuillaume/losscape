// Metric data as tiles (docs/REQUIREMENTS.md, Design 1). Level k splits the step axis
// into buckets of 2^k steps; a tile is TILE_BUCKETS consecutive buckets. Bucket bounds
// don't depend on the view, so tiles are cached and reused across zooms; buckets carry
// combinable aggregates, so live points can be folded into cached tiles.

import { onRestored } from '#lib/connection.svelte.js';
import { apiFetch } from '#lib/api/client.js';

export const TILE_BUCKETS = 1024;
const MAX_TILES_PER_RANGE = 64;
const MAX_RANGES_PER_REQUEST = 512;
const MAX_TILES_PER_REQUEST = 2048;
/** How many levels up to look for a coarser tile to show while a finer one loads. */
const FALLBACK_LEVELS = 9;
/** Live points kept per metric, to apply to tiles whose fetch raced with them. */
const LIVE_TAIL = 5_000;
const ROW_BYTES = 64;

/** Non-empty buckets of one tile, sorted by bucket index. */
export type Tile = {
	bucket: number[];
	n: number[];
	min: number[];
	max: number[];
	sum: number[];
	last: number[];
	/** Mean timestamp (epoch ms, client clock). */
	time: number[];
};

export type SeriesPoints = {
	/** Bucket centers, in steps. */
	x: number[];
	/** Finite points per bucket. */
	n: number[];
	/** Mean timestamp per bucket (epoch ms). */
	time: number[];
	mean: (number | null)[];
	min: (number | null)[];
	max: (number | null)[];
	/** All tiles at the wanted level are loaded. */
	complete: boolean;
};

type LivePoint = { step: number; value: number; time: number; at: number };
type Queued = { run: string; key: string; level: number; indices: Set<number> };

const SEP = '\u0000';
const seriesId = (run: string, key: string) => `${run}${SEP}${key}`;
const tileId = (series: string, level: number, index: number) =>
	`${series}${SEP}${level}${SEP}${index}`;
/** Requests within what the server accepts. */
export function splitRanges<R extends { from_tile: number; to_tile: number }>(ranges: R[]): R[][] {
	const requests: R[][] = [];
	let current: R[] = [];
	let tiles = 0;
	for (const range of ranges) {
		const size = range.to_tile - range.from_tile;
		if (current.length === MAX_RANGES_PER_REQUEST || tiles + size > MAX_TILES_PER_REQUEST) {
			requests.push(current);
			current = [];
			tiles = 0;
		}
		current.push(range);
		tiles += size;
	}
	if (current.length) requests.push(current);
	return requests;
}

export const tileSpan = (level: number) => TILE_BUCKETS * 2 ** level;

/** Finest level that still gives at most ~2 buckets per pixel. */
export function levelFor(steps: number, widthPx: number): number {
	const target = Math.max(1, 2 * widthPx);
	return steps <= target ? 0 : Math.ceil(Math.log2(steps / target));
}

function emptyTile(): Tile {
	return { bucket: [], n: [], min: [], max: [], sum: [], last: [], time: [] };
}

/** Index of `bucket` in the sorted tile, or where it would be inserted. */
function search(tile: Tile, bucket: number): number {
	let lo = 0;
	let hi = tile.bucket.length;
	while (lo < hi) {
		const mid = (lo + hi) >> 1;
		if (tile.bucket[mid] < bucket) lo = mid + 1;
		else hi = mid;
	}
	return lo;
}

function fold(tile: Tile, bucket: number, value: number, time: number) {
	const i = search(tile, bucket);
	if (tile.bucket[i] === bucket) {
		tile.time[i] += (time - tile.time[i]) / (tile.n[i] + 1);
		tile.n[i] += 1;
		tile.min[i] = Math.min(tile.min[i], value);
		tile.max[i] = Math.max(tile.max[i], value);
		tile.sum[i] += value;
		tile.last[i] = value;
	} else {
		tile.bucket.splice(i, 0, bucket);
		tile.n.splice(i, 0, 1);
		tile.min.splice(i, 0, value);
		tile.max.splice(i, 0, value);
		tile.sum.splice(i, 0, value);
		tile.last.splice(i, 0, value);
		tile.time.splice(i, 0, time);
	}
}

/** A metric over every point a run logged (finite ones): for aggregates in formulas. */
export type MetricStats = {
	n: number;
	min: number | null;
	max: number | null;
	mean: number | null;
	median: number | null;
};
/** The nearest finite point beyond a step (see `TileStore.neighbor`). */
export type NeighborPoint = { step: number; value: number; time: number };
/** Lookups per neighbors request (the server's limit). */
const MAX_NEIGHBOR_ITEMS = 200;

/** Items per statistics request (the server's limit). */
const MAX_STATS_ITEMS = 500;
/** A live run's statistics are asked again at most this often (ms): the median is costly. */
const STATS_REFRESH_MS = 15_000;

class TileStore {
	private tiles = new Map<string, Tile>();
	/** Statistics per metric: null when the run has no finite value of it. */
	private statsCache = new Map<string, { stats: MetricStats | null; at: number; stale: boolean }>();
	private statsQueue = new Map<string, { run: string; key: string }>();
	private statsInFlight = new Set<string>();
	private statsTimer: ReturnType<typeof setTimeout> | null = null;
	/** Nearest points beyond a step, per metric, side and step: null when there's none. */
	private neighborCache = new Map<string, NeighborPoint | null>();
	private neighborQueue = new Map<
		string,
		{ run: string; key: string; step: number; side: 'before' | 'after' }
	>();
	private neighborInFlight = new Set<string>();
	private neighborTimer: ReturnType<typeof setTimeout> | null = null;
	/** Levels with cached tiles, per metric (live points are folded into these). */
	private levels = new Map<string, Set<number>>();
	private inFlight = new Set<string>();
	private queue = new Map<string, Queued>();
	private flushTimer: ReturnType<typeof setTimeout> | null = null;
	private liveTail = new Map<string, LivePoint[]>();
	private notifyFrame: number | null = null;
	/** Bumped (at most once per frame) when tiles change; read it to re-render. */
	version = $state(0);

	private changed() {
		if (this.notifyFrame !== null) return;
		this.notifyFrame = requestAnimationFrame(() => {
			this.notifyFrame = null;
			this.version += 1;
		});
	}

	tile(run: string, key: string, level: number, index: number): Tile | undefined {
		return this.tiles.get(tileId(seriesId(run, key), level, index));
	}

	/** Queues tiles `from..to` (exclusive) that are neither cached nor loading. */
	request(run: string, key: string, level: number, from: number, to: number) {
		const series = seriesId(run, key);
		const groupId = `${series}${SEP}${level}`;
		for (let index = from; index < to; index++) {
			const id = tileId(series, level, index);
			if (this.tiles.has(id) || this.inFlight.has(id)) continue;
			let group = this.queue.get(groupId);
			if (!group) this.queue.set(groupId, (group = { run, key, level, indices: new Set() }));
			group.indices.add(index);
		}
		if (this.queue.size && this.flushTimer === null) {
			this.flushTimer = setTimeout(() => void this.flush(), 0);
		}
	}

	private async flush() {
		this.flushTimer = null;
		const ranges: {
			run: string;
			key: string;
			level: number;
			from_tile: number;
			to_tile: number;
		}[] = [];
		for (const { run, key, level, indices } of this.queue.values()) {
			const sorted = [...indices].sort((a, b) => a - b);
			let start = 0;
			for (let i = 1; i <= sorted.length; i++) {
				const contiguous = i < sorted.length && sorted[i] === sorted[i - 1] + 1;
				if (!contiguous || i - start === MAX_TILES_PER_RANGE) {
					ranges.push({ run, key, level, from_tile: sorted[start], to_tile: sorted[i - 1] + 1 });
					start = i;
				}
			}
		}
		this.queue.clear();
		for (const request of splitRanges(ranges)) void this.load(request);
	}

	private async load(
		ranges: { run: string; key: string; level: number; from_tile: number; to_tile: number }[]
	) {
		const ids = ranges.flatMap((r) => {
			const series = seriesId(r.run, r.key);
			return Array.from({ length: r.to_tile - r.from_tile }, (_, i) =>
				tileId(series, r.level, r.from_tile + i)
			);
		});
		ids.forEach((id) => this.inFlight.add(id));
		const issuedAt = performance.now();
		try {
			const res = await apiFetch('/series?format=rowbinary', {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ ranges })
			});
			const view = new DataView(await res.arrayBuffer());

			const loaded = new Map<string, Tile>(ids.map((id) => [id, emptyTile()]));
			// RowBinary, little-endian: req u32, bucket f64, n u32, then min, max, sum, first,
			// last, mean time as f64 (see crates/losscape-server/src/read.rs).
			for (let at = 0; at + ROW_BYTES <= view.byteLength; at += ROW_BYTES) {
				const range = ranges[view.getUint32(at, true)];
				const bucket = view.getFloat64(at + 4, true);
				const tile = loaded.get(
					tileId(seriesId(range.run, range.key), range.level, Math.floor(bucket / TILE_BUCKETS))
				);
				if (!tile) continue;
				tile.bucket.push(bucket);
				tile.n.push(view.getUint32(at + 12, true));
				tile.min.push(view.getFloat64(at + 16, true));
				tile.max.push(view.getFloat64(at + 24, true));
				tile.sum.push(view.getFloat64(at + 32, true));
				tile.last.push(view.getFloat64(at + 48, true));
				tile.time.push(view.getFloat64(at + 56, true));
			}
			for (const range of ranges) {
				const series = seriesId(range.run, range.key);
				// eslint-disable-next-line svelte/prefer-svelte-reactivity -- bookkeeping, never rendered
				const levels = this.levels.get(series) ?? new Set<number>();
				levels.add(range.level);
				this.levels.set(series, levels);
				const span = tileSpan(range.level);
				// Points pushed after the request was sent may not be in the response.
				for (const point of this.liveTail.get(series) ?? []) {
					if (point.at <= issuedAt) continue;
					const index = Math.floor(point.step / span);
					const tile = loaded.get(tileId(series, range.level, index));
					if (tile && index >= range.from_tile && index < range.to_tile) {
						fold(tile, Math.floor(point.step / 2 ** range.level), point.value, point.time);
					}
				}
			}
			for (const [id, tile] of loaded) this.tiles.set(id, tile);
			ids.forEach((id) => this.inFlight.delete(id));
			this.changed();
		} catch (err) {
			// Not asking again right away, which is what announcing a change would do: charts
			// ask for what they're missing. That's for when the connection is back.
			console.warn('losscape: loading tiles failed', err);
			ids.forEach((id) => this.inFlight.delete(id));
		}
	}

	/**
	 * A metric's statistics over the whole run: undefined until known (then asked for), null if
	 * it has no value. Out-of-date ones (a live run logged more) are given while asked again.
	 */
	stats(run: string, key: string): MetricStats | null | undefined {
		const id = seriesId(run, key);
		const cached = this.statsCache.get(id);
		const due = !cached || (cached.stale && performance.now() - cached.at > STATS_REFRESH_MS);
		if (due && !this.statsInFlight.has(id)) {
			this.statsQueue.set(id, { run, key });
			this.statsTimer ??= setTimeout(() => void this.loadStats(), 0);
		}
		return cached?.stats;
	}

	private async loadStats() {
		this.statsTimer = null;
		const all = [...this.statsQueue];
		this.statsQueue.clear();
		for (let i = 0; i < all.length; i += MAX_STATS_ITEMS) {
			const part = all.slice(i, i + MAX_STATS_ITEMS);
			part.forEach(([id]) => this.statsInFlight.add(id));
			try {
				const res = await apiFetch('/metric-stats', {
					method: 'POST',
					headers: { 'Content-Type': 'application/json' },
					body: JSON.stringify({ items: part.map(([, item]) => item) })
				});
				const rows: ({ run: string; key: string } & MetricStats)[] = await res.json();
				const found = new Map(rows.map((r) => [seriesId(r.run, r.key), r]));
				const at = performance.now();
				for (const [id] of part) {
					const r = found.get(id);
					this.statsCache.set(id, {
						stats: r ? { n: r.n, min: r.min, max: r.max, mean: r.mean, median: r.median } : null,
						at,
						stale: false
					});
				}
			} catch (err) {
				// Asked again later, like a live run's: not at every redraw.
				console.warn('losscape: loading metric statistics failed', err);
				const at = performance.now();
				for (const [id] of part) {
					this.statsCache.set(id, {
						stats: this.statsCache.get(id)?.stats ?? null,
						at,
						stale: true
					});
				}
			} finally {
				part.forEach(([id]) => this.statsInFlight.delete(id));
			}
		}
		this.changed();
	}

	/**
	 * The last finite point before `step`, or the first after it, however far: what a line
	 * crossing a view between two points is drawn to. Undefined until known (then asked for).
	 */
	neighbor(
		run: string,
		key: string,
		step: number,
		side: 'before' | 'after'
	): NeighborPoint | null | undefined {
		const id = `${seriesId(run, key)}${SEP}${side}${SEP}${step}`;
		if (this.neighborCache.has(id)) return this.neighborCache.get(id);
		if (!this.neighborInFlight.has(id)) {
			this.neighborQueue.set(id, { run, key, step, side });
			this.neighborTimer ??= setTimeout(() => void this.loadNeighbors(), 0);
		}
		return undefined;
	}

	private async loadNeighbors() {
		this.neighborTimer = null;
		const all = [...this.neighborQueue];
		this.neighborQueue.clear();
		for (let i = 0; i < all.length; i += MAX_NEIGHBOR_ITEMS) {
			const part = all.slice(i, i + MAX_NEIGHBOR_ITEMS);
			part.forEach(([id]) => this.neighborInFlight.add(id));
			try {
				const res = await apiFetch('/neighbors', {
					method: 'POST',
					headers: { 'Content-Type': 'application/json' },
					body: JSON.stringify({ items: part.map(([, item]) => item) })
				});
				const found: { item: number; step: number; value: number; time_ms: number }[] =
					await res.json();
				const byItem = new Map(found.map((n) => [n.item, n]));
				part.forEach(([id], k) => {
					const n = byItem.get(k);
					this.neighborCache.set(id, n ? { step: n.step, value: n.value, time: n.time_ms } : null);
				});
			} catch (err) {
				// Drawn without: the view still shows what it has.
				console.warn('losscape: loading neighbouring points failed', err);
				part.forEach(([id]) => this.neighborCache.set(id, null));
			} finally {
				part.forEach(([id]) => this.neighborInFlight.delete(id));
			}
		}
		this.changed();
	}

	/** Has charts ask again for what they're missing. */
	retry() {
		this.changed();
	}

	/** Adds a live point to every cached tile that covers it. */
	addLivePoint(run: string, key: string, step: number, value: number, time: number) {
		if (!Number.isFinite(value)) return;
		const series = seriesId(run, key);
		const tail = this.liveTail.get(series) ?? [];
		tail.push({ step, value, time, at: performance.now() });
		if (tail.length > LIVE_TAIL) tail.splice(0, tail.length - LIVE_TAIL);
		this.liveTail.set(series, tail);
		const stats = this.statsCache.get(series);
		if (stats) stats.stale = true;
		// A new point can be the nearest one after a step (not before: it's the latest).
		const after = `${series}${SEP}after${SEP}`;
		for (const id of this.neighborCache.keys())
			if (id.startsWith(after)) this.neighborCache.delete(id);
		for (const level of this.levels.get(series) ?? []) {
			const tile = this.tiles.get(tileId(series, level, Math.floor(step / tileSpan(level))));
			if (tile) fold(tile, Math.floor(step / 2 ** level), value, time);
		}
		this.changed();
	}

	/** Forgets a run's tiles (after missed live events). */
	invalidate(run: string) {
		const prefix = `${run}${SEP}`;
		for (const id of this.tiles.keys()) if (id.startsWith(prefix)) this.tiles.delete(id);
		for (const id of this.levels.keys()) if (id.startsWith(prefix)) this.levels.delete(id);
		for (const id of this.liveTail.keys()) if (id.startsWith(prefix)) this.liveTail.delete(id);
		for (const id of this.statsCache.keys()) if (id.startsWith(prefix)) this.statsCache.delete(id);
		for (const id of this.neighborCache.keys())
			if (id.startsWith(prefix)) this.neighborCache.delete(id);
		this.changed();
	}
}

export const tileStore = new TileStore();
onRestored(() => tileStore.retry());

/**
 * Points of one metric for steps `from..=to` drawn `widthPx` wide: bucket means plus the
 * min/max band. Requests what's missing; meanwhile fills gaps from coarser cached tiles.
 */
export function seriesInRange(
	run: string,
	key: string,
	from: number,
	to: number,
	widthPx: number
): SeriesPoints {
	const level = levelFor(to - from + 1, widthPx);
	const span = tileSpan(level);
	const first = Math.floor(from / span);
	const last = Math.floor(to / span);
	tileStore.request(run, key, level, first, last + 1);

	const out: SeriesPoints = {
		x: [],
		n: [],
		time: [],
		mean: [],
		min: [],
		max: [],
		complete: true
	};
	for (let index = first; index <= last; index++) {
		let tile = tileStore.tile(run, key, level, index);
		let tileLevel = level;
		if (!tile) {
			out.complete = false;
			for (let up = 1; up <= FALLBACK_LEVELS && !tile; up++) {
				tile = tileStore.tile(
					run,
					key,
					level + up,
					Math.floor((index * span) / tileSpan(level + up))
				);
				tileLevel = level + up;
			}
		}
		if (!tile) continue;
		const width = 2 ** tileLevel;
		const [lo, hi] = [index * span, (index + 1) * span];
		for (let j = 0; j < tile.bucket.length; j++) {
			const start = tile.bucket[j] * width;
			if (start < lo || start >= hi) continue; // part of a neighbouring tile (coarse fallback)
			const n = tile.n[j];
			out.x.push(width === 1 ? start : start + (width - 1) / 2);
			out.n.push(n);
			out.time.push(tile.time[j]);
			out.mean.push(n > 0 ? tile.sum[j] / n : null);
			out.min.push(n > 0 ? tile.min[j] : null);
			out.max.push(n > 0 ? tile.max[j] : null);
		}
	}
	return out;
}
