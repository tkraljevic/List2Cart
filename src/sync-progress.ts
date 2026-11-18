export interface SyncProgress {
  title: string;
  steps: string[];
  current: number;
  completed: number;
  state: "running" | "done" | "error";
  committed: boolean;
  detail: string;
  updatedAt: number;
}

export interface SyncStep {
  label: string;
  action: () => Promise<void>;
  commitsData?: boolean;
}

export class SyncTracker {
  progress: SyncProgress | null = null;
  constructor(private readonly changed: () => void) {}

  async run(title: string, steps: SyncStep[], success: () => string): Promise<boolean> {
    if (this.progress?.state === "running") return false;
    this.progress = {title, steps: steps.map(s => s.label), current: 0, completed: 0,
      state: "running", committed: false, detail: "", updatedAt: Date.now()};
    this.changed();
    try {
      for (const [index, step] of steps.entries()) {
        this.progress.current = index;
        this.progress.updatedAt = Date.now();
        this.changed();
        await step.action();
        if (step.commitsData) this.progress.committed = true;
        this.progress.completed = index + 1;
      }
      this.progress.state = "done";
      this.progress.detail = success();
      return true;
    } catch (error) {
      this.progress.state = "error";
      this.progress.detail = `${this.progress.committed ? "Podaci su spremljeni, ali sljedeći korak nije uspio. " : ""}${String(error)}`;
      return false;
    } finally {
      this.progress.updatedAt = Date.now();
      this.changed();
    }
  }
}
