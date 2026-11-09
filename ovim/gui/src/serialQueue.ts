/**
 * Runs async tasks one at a time, in the order they were queued. A failed
 * task does not stop the tasks queued after it.
 */
export const createSerialQueue = () => {
    let tail: Promise<unknown> = Promise.resolve();
    return <T>(task: () => Promise<T>): Promise<T> => {
        const result = tail.then(task);
        tail = result.catch(() => undefined);
        return result;
    };
};
