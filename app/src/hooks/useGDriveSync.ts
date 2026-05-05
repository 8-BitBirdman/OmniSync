import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface SyncStatus {
    state: "disconnected" | "connecting" | "connected" | "syncing" | "error";
    message: string;
    lastSynced: string | null;
    filesSynced: number;
}

export interface SyncEvent {
    eventType: "file_synced" | "error" | "status_changed";
    fileName: string | null;
    message: string;
}

export function useGDriveSync() {
    const [status, setStatus] = useState<SyncStatus>({
        state: "disconnected",
        message: "Not connected",
        lastSynced: null,
        filesSynced: 0,
    });
    const [recentEvents, setRecentEvents] = useState<SyncEvent[]>([]);

    // Fetch initial status on mount
    useEffect(() => {
        invoke<SyncStatus>("cmd_gdrive_sync_status")
            .then(setStatus)
            .catch(() => {/* not connected yet */});
    }, []);

    // Listen for realtime status pushes from Rust
    useEffect(() => {
        const unlistenStatus = listen<SyncStatus>("gdrive-status", (event) => {
            setStatus(event.payload);
        });
        const unlistenEvents = listen<SyncEvent>("gdrive-sync-event", (event) => {
            setRecentEvents(prev => [event.payload, ...prev].slice(0, 20));
        });
        return () => {
            unlistenStatus.then(f => f());
            unlistenEvents.then(f => f());
        };
    }, []);

    const disconnect = useCallback(async () => {
        await invoke("cmd_gdrive_disconnect");
    }, []);

    const isConnected = status.state === "connected" || status.state === "syncing";

    return { status, recentEvents, disconnect, isConnected };
}
