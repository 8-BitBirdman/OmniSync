import { useEffect, useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { Store } from "@tauri-apps/plugin-store";
import { AuthWizard } from "./components/AuthWizard";
import { Dashboard } from "./components/Dashboard";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { UpdateBanner } from "./components/UpdateBanner";
import { GoogleDriveConnect } from "./components/GoogleDriveConnect";
import { useUpdateCheck } from "./hooks/useUpdateCheck";
import "./App.css";

import { Toaster } from "sonner";
import { ConfirmProvider } from "./context/ConfirmContext";
import { ThemeProvider, useTheme } from "./context/ThemeContext";
import { DropZoneProvider } from "./contexts/DropZoneContext";

type AppStep = "boot" | "auth" | "gdrive" | "dashboard";

const queryClient = new QueryClient({
    defaultOptions: {
        queries: { retry: false, refetchOnWindowFocus: false },
        mutations: { retry: false },
    },
});

const STORE_FILE = "config.json";
const LEGACY_STORE_FILE = "settings.json";
const GDRIVE_SETUP_DONE_KEY = "gdrive_setup_done";

async function loadActiveStore(): Promise<Store> {
    // Prefer config.json; fall back to legacy settings.json if it has data.
    const primary = await Store.load(STORE_FILE);
    const apiId = await primary.get<string>("api_id");
    if (apiId) return primary;
    return Store.load(LEGACY_STORE_FILE);
}

function AppContent() {
    const [step, setStep] = useState<AppStep>("boot");
    const { theme } = useTheme();
    const { available, version, downloading, progress, downloadAndInstall, dismissUpdate } = useUpdateCheck();

    // Cold-start bootstrap: decide which screen to show before mounting Dashboard.
    // Previously the auto-reconnect lived inside Dashboard, so a returning user
    // was forced through AuthWizard + GoogleDriveConnect every launch.
    useEffect(() => {
        let cancelled = false;
        (async () => {
            try {
                const store = await loadActiveStore();
                const apiIdStr = await store.get<string>("api_id");
                if (!apiIdStr) {
                    if (!cancelled) setStep("auth");
                    return;
                }

                const apiId = parseInt(apiIdStr, 10);
                if (Number.isNaN(apiId)) {
                    if (!cancelled) setStep("auth");
                    return;
                }

                try {
                    await invoke("cmd_connect", { apiId });
                } catch {
                    // Connection failed — wipe creds and force re-auth.
                    await store.delete("api_id");
                    await store.delete("api_hash");
                    await store.save();
                    if (!cancelled) setStep("auth");
                    return;
                }

                const gdriveDone = await store.get<boolean>(GDRIVE_SETUP_DONE_KEY);
                if (!cancelled) setStep(gdriveDone ? "dashboard" : "gdrive");
            } catch {
                if (!cancelled) setStep("auth");
            }
        })();
        return () => { cancelled = true; };
    }, []);

    const markGDriveDone = async () => {
        try {
            const store = await loadActiveStore();
            await store.set(GDRIVE_SETUP_DONE_KEY, true);
            await store.save();
        } catch {
            // Non-fatal — user just sees the screen again next launch.
        }
        setStep("dashboard");
    };

    const handleLogout = async () => {
        try {
            const store = await loadActiveStore();
            await store.delete(GDRIVE_SETUP_DONE_KEY);
            await store.save();
        } catch {
            // ignore
        }
        setStep("auth");
    };

    return (
        <main className="h-screen w-screen text-telegram-text overflow-hidden selection:bg-telegram-primary/30 relative">
            <UpdateBanner
                available={available}
                version={version}
                downloading={downloading}
                progress={progress}
                onUpdate={downloadAndInstall}
                onDismiss={dismissUpdate}
            />
            <Toaster theme={theme} position="bottom-center" />

            {step === "boot" && (
                <div className="h-full w-full flex items-center justify-center text-telegram-subtext text-sm">
                    Loading…
                </div>
            )}
            {step === "auth" && (
                <AuthWizard onLogin={() => setStep("gdrive")} />
            )}
            {step === "gdrive" && (
                <GoogleDriveConnect
                    onConnected={markGDriveDone}
                    onSkip={markGDriveDone}
                />
            )}
            {step === "dashboard" && (
                <Dashboard onLogout={handleLogout} />
            )}
        </main>
    );
}

function App() {
    return (
        <ErrorBoundary>
            <ThemeProvider>
                <QueryClientProvider client={queryClient}>
                    <ConfirmProvider>
                        <DropZoneProvider>
                            <AppContent />
                        </DropZoneProvider>
                    </ConfirmProvider>
                </QueryClientProvider>
            </ThemeProvider>
        </ErrorBoundary>
    );
}

export default App;
