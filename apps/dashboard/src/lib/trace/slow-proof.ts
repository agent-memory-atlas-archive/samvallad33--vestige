/**
 * One lane for the slow proofs (receipt re-verification, the frame that wrote
 * a memory). On a Strata log each holds the store for seconds and every other
 * read queues behind it, so only one runs at a time: a newer request replaces
 * one still waiting, and a request whose memory is no longer shown is dropped.
 */
let running = false;
let waiting: { stale: () => boolean; job: () => Promise<unknown> } | null = null;

export function slowProof(stale: () => boolean, job: () => Promise<unknown>): void {
	waiting = { stale, job };
	if (!running) void drain();
}

async function drain(): Promise<void> {
	while (waiting) {
		const next = waiting;
		waiting = null;
		if (next.stale()) continue;
		running = true;
		try {
			await next.job();
		} catch {
			// Each job reports its own failure to its card.
		} finally {
			running = false;
		}
	}
}
