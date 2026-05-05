import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Store } from '@tauri-apps/plugin-store';
import { useQueryClient } from '@tanstack/react-query';
import { toast } from 'sonner';
import { useConfirm } from '../context/ConfirmContext';
import { TelegramFolder } from '../types';
import { useNetworkStatus } from './useNetworkStatus';

export function useTelegramConnection(onLogoutParent: () => void) {
    const queryClient = useQueryClient();
    const { confirm } = useConfirm();

    const [folders, setFolders] = useState<TelegramFolder[]>([]);
    const [activeFolderId, setActiveFolderId] = useState<number | null>(null);
    const [store, setStore] = useState<Store | null>(null);
    const [isSyncing, setIsSyncing] = useState(false);
    const [isConnected, setIsConnected] = useState(true);


    const networkIsOnline = useNetworkStatus();

    // Always read the latest parent callback via a ref so the init effect doesn't
    // re-run every render just because the parent passes a fresh arrow fn.
    const onLogoutParentRef = useRef(onLogoutParent);
    useEffect(() => { onLogoutParentRef.current = onLogoutParent; }, [onLogoutParent]);


    useEffect(() => {
        // App.tsx already ran cmd_connect on cold start. Here we only need to
        // load the store + persisted UI state (folders, last active folder).
        let cancelled = false;
        const initStore = async () => {
            try {
                let _store = await Store.load('config.json');
                const checkId = await _store.get<string>('api_id');
                if (!checkId) {
                    _store = await Store.load('settings.json');
                }
                if (cancelled) return;
                setStore(_store);

                const savedFolders = await _store.get<TelegramFolder[]>('folders');
                if (!cancelled && savedFolders) setFolders(savedFolders);

                const savedActiveFolderId = await _store.get<number | null>('activeFolderId');
                if (!cancelled && savedActiveFolderId !== undefined) {
                    setActiveFolderId(savedActiveFolderId);
                }

                if (!cancelled) {
                    queryClient.invalidateQueries({ queryKey: ['files'] });
                }
            } catch {
                // store not available — non-fatal
            }
        };
        initStore();
        return () => { cancelled = true; };
        // queryClient is stable; confirm no longer needed here.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);


    useEffect(() => {
        setIsConnected(networkIsOnline);
    }, [networkIsOnline]);


    const handleLogout = async () => {
        if (!await confirm({ title: "Sign Out", message: "Are you sure you want to sign out? This will disconnect your active session.", confirmText: "Sign Out", variant: 'danger' })) return;

        try {
            await invoke('cmd_logout');
            await invoke('cmd_clean_cache');
            if (store) {
                await store.delete('api_id');
                await store.delete('api_hash');
                await store.delete('folders');
                await store.delete('activeFolderId');
                await store.delete('uploadQueue');
                await store.delete('downloadQueue');
                await store.save();
            }
            onLogoutParent();
        } catch {
            toast.error("Error signing out");
            onLogoutParent();
        }
    };

    const handleSyncFolders = async () => {
        if (!store) return;
        setIsSyncing(true);
        try {
            const foundFolders = await invoke<TelegramFolder[]>('cmd_scan_folders');
            const merged = [...folders];
            let added = 0;
            for (const f of foundFolders) {
                if (!merged.find(existing => existing.id === f.id)) {
                    merged.push(f);
                    added++;
                }
            }
            if (added > 0) {
                setFolders(merged);
                await store.set('folders', merged);
                await store.save();
                toast.success(`Scan complete. Found ${added} new folders.`);
            } else {
                toast.info("Scan complete. No new folders found.");
            }
        } catch {
            toast.error("Sync failed");
        } finally {
            setIsSyncing(false);
        }
    };

    const handleCreateFolder = async (name: string) => {
        if (!store) return;
        try {
            const newFolder = await invoke<TelegramFolder>('cmd_create_folder', { name });
            const updated = [...folders, newFolder];
            setFolders(updated);
            await store.set('folders', updated);
            await store.save();
            toast.success(`Folder "${name}" created.`);
        } catch (e) {
            toast.error("Failed to create folder: " + e);
            throw e;
        }
    };

    const handleFolderDelete = async (folderId: number, folderName: string) => {
        if (!await confirm({
            title: "Delete Folder",
            message: `Are you sure you want to delete "${folderName}"?\nThis will delete the channel on Telegram.`,
            confirmText: "Delete",
            variant: 'danger'
        })) return;

        try {
            await invoke('cmd_delete_folder', { folderId });
            const updated = folders.filter(f => f.id !== folderId);
            setFolders(updated);
            if (store) {
                await store.set('folders', updated);
                await store.save();
            }
            if (activeFolderId === folderId) setActiveFolderId(null);
            toast.success(`Folder "${folderName}" deleted.`);
        } catch (e: unknown) {
            const errStr = String(e);
            if (errStr.includes("not found")) {
                if (await confirm({
                    title: "Folder Not Found",
                    message: `Folder "${folderName}" not found on Telegram (it may have been deleted externally).\nRemove from this app?`,
                    confirmText: "Remove",
                    variant: 'info'
                })) {
                    const updated = folders.filter(f => f.id !== folderId);
                    setFolders(updated);
                    if (store) {
                        await store.set('folders', updated);
                        await store.save();
                    }
                    if (activeFolderId === folderId) setActiveFolderId(null);
                }
            } else {
                toast.error(`Failed to delete folder: ${e}`);
            }
        }
    };


    const handleSetActiveFolderId = async (id: number | null) => {
        setActiveFolderId(id);
        if (store) {
            await store.set('activeFolderId', id);
            await store.save();
        }
    };

    return {
        store,
        folders,
        activeFolderId,
        setActiveFolderId: handleSetActiveFolderId,
        isSyncing,
        isConnected,
        handleLogout,
        handleSyncFolders,
        handleCreateFolder,
        handleFolderDelete,
    };
}
