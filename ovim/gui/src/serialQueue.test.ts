import { describe, expect, it } from "vitest";
import { createSerialQueue } from "./serialQueue";

const deferred = <T>() => {
    let resolve!: (value: T) => void;
    let reject!: (reason: unknown) => void;
    const promise = new Promise<T>((onResolve, onReject) => {
        resolve = onResolve;
        reject = onReject;
    });
    return { promise, resolve, reject };
};

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("createSerialQueue", () => {
    it("starts a task only after the previous one has finished", async () => {
        const inOrder = createSerialQueue();
        const started: string[] = [];
        const first = deferred<string>();

        const a = inOrder(() => {
            started.push("a");
            return first.promise;
        });
        const b = inOrder(async () => {
            started.push("b");
            return "b";
        });
        await settle();
        expect(started).toEqual(["a"]);

        first.resolve("a");
        await expect(a).resolves.toBe("a");
        await expect(b).resolves.toBe("b");
        expect(started).toEqual(["a", "b"]);
    });

    it("keeps running later tasks after one fails", async () => {
        const inOrder = createSerialQueue();
        const failed = inOrder(() =>
            Promise.reject(new Error("editor stopped")),
        );
        const next = inOrder(async () => "next");

        await expect(failed).rejects.toThrow("editor stopped");
        await expect(next).resolves.toBe("next");
    });
});
