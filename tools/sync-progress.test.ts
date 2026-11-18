import {test, expect} from "bun:test";
import {SyncTracker} from "../src/sync-progress";

test("a failed import stops before refresh and keeps the failed phase", async () => {
  const tracker = new SyncTracker(() => {});
  let refreshed = false;
  expect(await tracker.run("Import", [
    {label:"Read",action:async () => {}},
    {label:"Apply",commitsData:true,action:async () => { throw Error("invalid"); }},
    {label:"Refresh",action:async () => {refreshed=true;}},
  ],() => "Done")).toBe(false);
  expect(refreshed).toBe(false);
  expect(tracker.progress?.current).toBe(1);
  expect(tracker.progress?.completed).toBe(1);
  expect(tracker.progress?.committed).toBe(false);
  expect(tracker.progress?.state).toBe("error");
});

test("a refresh failure reports committed data and another operation cannot overlap", async () => {
  const tracker = new SyncTracker(() => {});
  let release!: () => void;
  const pending = new Promise<void>(resolve => {release=resolve;});
  const run = tracker.run("Import", [
    {label:"Apply",commitsData:true,action:() => pending},
    {label:"Refresh",action:async () => {throw Error("refresh failed");}},
  ],() => "Done");
  expect(await tracker.run("Other",[],() => "Other")).toBe(false);
  release(); await run;
  expect(tracker.progress?.committed).toBe(true);
  expect(tracker.progress?.detail).toContain("Podaci su spremljeni");
  expect(await tracker.run("Retry",[{label:"Refresh",action:async () => {}}],() => "Updated")).toBe(true);
  expect(tracker.progress?.state).toBe("done");
});
