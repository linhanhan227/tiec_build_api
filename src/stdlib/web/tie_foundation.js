export class TaskScheduler {
  constructor() {
    this.timerMap = new Map();
    this.isRunning = false;
  }

  post(task) {
    if (typeof task !== 'function') {
      return;
    }
    const timerId = setTimeout(() => {
      task();
      this.timerMap.delete(task);
    }, 0);
    this.timerMap.set(task, timerId);
  }

  postDelayed(task, delayMs) {
    if (typeof task !== 'function') {
      return;
    }
    const timerId = setTimeout(() => {
      task();
      this.timerMap.delete(task);
    }, delayMs);
    this.timerMap.set(task, timerId);
  }

  postImmediate(task) {
    if (typeof task !== 'function') {
      return;
    }
    if (typeof queueMicrotask === "function") {
      queueMicrotask(task);
    } else {
      Promise.resolve().then(task);
    }
  }

  removeCallbacks(task) {
    if (this.timerMap.has(task)) {
      clearTimeout(this.timerMap.get(task));
      this.timerMap.delete(task);
    }
  }
}

export class Window {
  onInit() {
  }

  setRootLayout(component) {
    document.body.append(component.getElement())
    this.onCreate()
  }

  onCreate() {
  }
}