import { useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
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

type AppStep = "auth" | "gdrive" | "dashboard";

const queryClient = new QueryClient();

function AppContent() {
    const [step, setStep] = useState<AppStep>("auth");
    const { theme } = useTheme();
    const { available, version, downloading, progress, downloadAndInstall, dismissUpdate } = useUpdateCheck();

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

            {step === "auth" && (
                <AuthWizard onLogin={() => setStep("gdrive")} />
            )}
            {step === "gdrive" && (
                <GoogleDriveConnect
                    onConnected={() => setStep("dashboard")}
                    onSkip={() => setStep("dashboard")}
                />
            )}
            {step === "dashboard" && (
                <Dashboard onLogout={() => setStep("auth")} />
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
