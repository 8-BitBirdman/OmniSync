import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

/**
 * Network detection for Tauri apps using a lightweight backend check.
 *
 * Calls cmd_is_network_available which does a TCP connection test
 * to Telegram servers without using grammers (avoids stack overflow).
 *
 * Polls every 10 seconds — very lightweight (~2ms per check).
 */
export function useNetworkStatus() {
    const [isOnline, setIsOnline] = useState(true);

    useEffect(() => {
        let cancelled = false;

        const checkNetwork = async () => {
            try {
                const available = await invoke<boolean>('cmd_is_network_available');
                if (!cancelled) setIsOnline(available);
            } catch {
                if (!cancelled) setIsOnline(false);
            }
        };

        checkNetwork();
        const interval = setInterval(checkNetwork, 10000);
        return () => {
            cancelled = true;
            clearInterval(interval);
        };
    }, []);

    return isOnline;
}
