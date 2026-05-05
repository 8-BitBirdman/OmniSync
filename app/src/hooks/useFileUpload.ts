import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { useQueryClient } from '@tanstack/react-query';
import { toast } from 'sonner';
import { QueueItem } from '../types';
import { useFileDrop } from './useFileDrop';
import type { Store } from '@tauri-apps/plugin-store';

interface ProgressPayload {
    id: string;
    percent: number;
}

export function useFileUpload(activeFolderId: number | null, store: Store | null) {
    const queryClient = useQueryClient();
    const [uploadQueue, setUploadQueue] = useState<QueueItem[]>([]);
    const [processing, setProcessing] = useState(false);
    const [initialized, setInitialized] = useState(false);
    const cancelledRef = useRef<Set<string>>(new Set());

    // Listen for progress events from Rust
    useEffect(() => {
        let cancelled = false;
        let unlisten: UnlistenFn | undefined;
        listen<ProgressPayload>('upload-progress', (event) => {
            setUploadQueue(q => q.map(i =>
                i.id === event.payload.id ? { ...i, progress: event.payload.percent } : i
            ));
        }).then(fn => {
            // If we already unmounted before the listener registered, immediately tear it down.
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

    useEffect(() => {
        if (!store || initialized) return;
        store.get<QueueItem[]>('uploadQueue').then((saved) => {
            if (saved && saved.length > 0) {
                const pending = saved.filter(i => i.status === 'pending');
                if (pending.length > 0) {
                    setUploadQueue(pending);
                    toast.info(`Restored ${pending.length} pending uploads`);
                }
            }
            setInitialized(true);
        });
    }, [store, initialized]);

    const lastPendingKeyRef = useRef<string>("");
    useEffect(() => {
        if (!store || !initialized) return;
        const pending = uploadQueue.filter(i => i.status === 'pending');
        // Only hit disk when the *set of pending items* changes, not on every
        // progress tick (which fires many times per second).
        const key = pending.map(i => `${i.id}|${i.path}|${i.folderId}`).join('\n');
        if (key === lastPendingKeyRef.current) return;
        lastPendingKeyRef.current = key;
        store.set('uploadQueue', pending).then(() => store.save());
    }, [store, uploadQueue, initialized]);

    useEffect(() => {
        if (processing) return;
        const nextItem = uploadQueue.find(i => i.status === 'pending');
        if (nextItem) {
            processItem(nextItem);
        }
    }, [uploadQueue, processing, processItem]);

    const processItem = useCallback(async (item: QueueItem) => {
        setProcessing(true);
        setUploadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'uploading', progress: 0 } : i));
        try {
            await invoke('cmd_upload_file', { path: item.path, folderId: item.folderId, transferId: item.id });
            // Check if cancelled during upload
            if (cancelledRef.current.has(item.id)) {
                cancelledRef.current.delete(item.id);
            } else {
                setUploadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'success', progress: 100 } : i));
                queryClient.invalidateQueries({ queryKey: ['files', item.folderId] });
            }
        } catch (e) {
            if (!cancelledRef.current.has(item.id)) {
                setUploadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'error', error: String(e) } : i));
                toast.error(`Upload failed for ${item.path.split('/').pop()}: ${e}`);
            } else {
                cancelledRef.current.delete(item.id);
            }
        } finally {
            setProcessing(false);
        }
    }, [queryClient]);

    const handleManualUpload = async () => {
        try {
            const selected = await open({ multiple: true, directory: false });
            if (selected) {
                const paths = Array.isArray(selected) ? selected : [selected];
                const newItems: QueueItem[] = paths.map((path: string) => ({
                    id: crypto.randomUUID(),
                    path,
                    folderId: activeFolderId,
                    status: 'pending'
                }));
                setUploadQueue(prev => [...prev, ...newItems]);
                toast.info(`Queued ${paths.length} files for upload`);
            }
        } catch {
            toast.error("Failed to open file dialog");
        }
    };

    const cancelAll = () => {
        setUploadQueue(q => {
            const uploading = q.find(i => i.status === 'uploading');
            if (uploading) cancelledRef.current.add(uploading.id);
            return q
                .filter(i => i.status !== 'pending')
                .map(i => i.status === 'uploading' ? { ...i, status: 'cancelled' as const } : i);
        });
        toast.info('All uploads cancelled');
    };

    const { isDragging } = useFileDrop();

    return {
        uploadQueue,
        setUploadQueue,
        handleManualUpload,
        cancelAll,
        isDragging
    };
}
