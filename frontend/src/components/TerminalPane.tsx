import { createSignal, onCleanup, onMount } from "solid-js";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { subscribeToTerminal, type TerminalSession } from "../api/client";

export function TerminalPane(props: { taskId: string }) {
  const decoder = new TextDecoder();
  const encoder = new TextEncoder();
  const [connected, setConnected] = createSignal(false);
  let containerRef: HTMLDivElement | undefined;
  let session: TerminalSession | null = null;

  onMount(() => {
    if (!containerRef) {
      return;
    }

    const terminal = new Terminal({
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
      fontSize: 13,
      cursorBlink: true,
      convertEol: true,
      scrollback: 3000,
      theme: {
        background: "#0b1020",
        foreground: "#e6edf7",
        cursor: "#79c0ff",
      },
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(containerRef);
    fitAddon.fit();

    session = subscribeToTerminal(
      props.taskId,
      (chunk) => {
        terminal.write(decoder.decode(chunk, { stream: true }));
      },
      () => {
        setConnected(false);
        terminal.writeln("\r\n[terminal disconnected]");
      }
    );

    setConnected(true);
    terminal.focus();
    terminal.onData((data) => {
      session?.sendInput(encoder.encode(data));
    });
    terminal.onResize(({ rows, cols }) => {
      session?.resize(rows, cols);
    });
    session.resize(terminal.rows, terminal.cols);

    const resizeObserver = new ResizeObserver(() => {
      fitAddon.fit();
      session?.resize(terminal.rows, terminal.cols);
    });
    resizeObserver.observe(containerRef);
    const onWindowResize = () => {
      fitAddon.fit();
      session?.resize(terminal.rows, terminal.cols);
    };
    window.addEventListener("resize", onWindowResize);

    onCleanup(() => {
      window.removeEventListener("resize", onWindowResize);
      resizeObserver.disconnect();
      session?.close();
      session = null;
      terminal.dispose();
    });
  });

  return (
    <div class="flex h-full min-h-0 flex-col">
      <div class="mb-2 text-xs text-gray-500 dark:text-gray-400">
        {connected() ? "Connected terminal in environment directory." : "Connecting terminal..."}
      </div>
      <div ref={containerRef} class="flex-1 min-h-0 overflow-hidden rounded-lg border border-gray-700" />
    </div>
  );
}
