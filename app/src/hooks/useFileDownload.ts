import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { save, open } from '@tauri-apps/plugin-dialog';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { DownloadItem, TelegramFile } from '../types';
import type { Store } from '@tauri-apps/plugin-store';

interface ProgressPayload {
    id: string;
    percent: number;
}

const sanitizeFilename = (name: string): string => {
    const cleaned = name.replace(/[\\/]/g, '_').replace(/^\.+/, '_').trim();
    return cleaned.length > 0 ? cleaned.slice(0, 200) : 'file';
};

export function useFileDownload(store: Store | null) {
    const [downloadQueue, setDownloadQueue] = useState<DownloadItem[]>([]);
    const [processing, setProcessing] = useState(false);
    const [initialized, setInitialized] = useState(false);
    const cancelledRef = useRef<Set<string>>(new Set());

    // Listen for progress events from Rust
    useEffect(() => {
        let cancelled = false;
        let unlisten: UnlistenFn | undefined;
        listen<ProgressPayload>('download-progress', (event) => {
            setDownloadQueue(q => q.map(i =>
                i.id === event.payload.id ? { ...i, progress: event.payload.percent } : i
            ));
        }).then(fn => {
            if (cancelled) {
                fn();
            } else {
                unlisten = fn;
            }
        });
        return () => {
            cancelled = true;
            unlisten?.();
        };
    }, []);

    // Load saved queue on mount
    useEffect(() => {
        if (!store || initialized) return;
        store.get<DownloadItem[]>('downloadQueue').then((saved) => {
            if (saved && saved.length > 0) {
                const pending = saved.filter(i => i.status === 'pending');
                if (pending.length > 0) {
                    setDownloadQueue(pending);
                    toast.info(`Restored ${pending.length} pending downloads`);
                }
            }
            setInitialized(true);
        });
    }, [store, initialized]);

    // Debounced persist — only write when the *set of pending items* changes.
    const lastPendingKeyRef = useRef<string>("");
    useEffect(() => {
        if (!store || !initialized) return;
        const pending = downloadQueue.filter(i => i.status === 'pending');
        const key = pending.map(i => `${i.id}|${i.messageId}|${i.folderId}|${i.dirPath ?? ''}`).join('\n');
        if (key === lastPendingKeyRef.current) return;
        lastPendingKeyRef.current = key;
        store.set('downloadQueue', pending).then(() => store.save());
    }, [store, downloadQueue, initialized]);

    const processItem = useCallback(async (item: DownloadItem) => {
        setProcessing(true);
        setDownloadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'downloading', progress: 0 } : i));

        try {
            // Resolve final save path: prefer pre-selected dirPath, else open save dialog.
            let savePath: string | null = null;
            if (item.dirPath) {
                savePath = `${item.dirPath}/${sanitizeFilename(item.filename)}`;
            } else {
                savePath = await save({ defaultPath: item.filename });
            }
            if (!savePath) {
                setDownloadQueue(q => q.filter(i => i.id !== item.id));
                setProcessing(false);
                return;
            }

            await invoke('cmd_download_file', {
                messageId: item.messageId,
                savePath,
                folderId: item.folderId,
                transferId: item.id
            });

            if (cancelledRef.current.has(item.id)) {
                cancelledRef.current.delete(item.id);
            } else {
                setDownloadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'success', progress: 100 } : i));
                toast.success(`Downloaded: ${item.filename}`);
            }
        } catch (e) {
            if (!cancelledRef.current.has(item.id)) {
                setDownloadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'error', error: String(e) } : i));
                toast.error(`Download failed: ${item.filename}`);
            } else {
                cancelledRef.current.delete(item.id);
            }
        } finally {
            setProcessing(false);
        }
    }, []);

    // Queue Processor
    useEffect(() => {
        if (processing) return;
        const nextItem = downloadQueue.find(i => i.status === 'pending');
        if (nextItem) {
            processItem(nextItem);
        }
    }, [downloadQueue, processing, processItem]);

    const queueDownload = (messageId: number, filename: string, folderId: number | null) => {
        const newItem: DownloadItem = {
            id: crypto.randomUUID(),
            messageId,
            filename,
            folderId,
            status: 'pending'
        };
        setDownloadQueue(prev => [...prev, newItem]);
    };

    const queueBulkDownload = async (files: TelegramFile[], folderId: number | null) => {
        const dirPath = await open({
            directory: true,
            multiple: false,
            title: "Select Download Destination"
        });
        if (!dirPath) return;

        for (const file of files) {
            const newItem: DownloadItem = {
                id: crypto.randomUUID(),
                messageId: file.id,
                filename: file.name,
                folderId,
                dirPath: dirPath as string,
                status: 'pending'
            };
            setDownloadQueue(prev => [...prev, newItem]);
        }

        toast.info(`Queued ${files.length} files for download`);
    };

    const clearFinished = () => {
        setDownloadQueue(q => q.filter(i => i.status !== 'success'));
    };

    const cancelAll = () => {
        setDownloadQueue(q => {
            const downloading = q.find(i => i.status === 'downloading');
            if (downloading) cancelledRef.current.add(downloading.id);
            return q
                .filter(i => i.status !== 'pending')
                .map(i => i.status === 'downloading' ? { ...i, status: 'cancelled' as const } : i);
        });
        toast.info('All downloads cancelled');
    };

    return {
        downloadQueue,
        queueDownload,
        queueBulkDownload,
        clearFinished,
        cancelAll
    };
}
