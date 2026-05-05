import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { motion, AnimatePresence } from "framer-motion";
import { CheckCircle2, ChevronRight, ExternalLink, Info, Loader2, X } from "lucide-react";

interface Props {
    onConnected: () => void;
    onSkip: () => void;
}

export function GoogleDriveConnect({ onConnected, onSkip }: Props) {
    const [step, setStep] = useState<"intro" | "credentials" | "connecting" | "done">("intro");
    const [clientId, setClientId] = useState("");
    const [clientSecret, setClientSecret] = useState("");
    const [error, setError] = useState<string | null>(null);
    const [showHelp, setShowHelp] = useState(false);

    async function handleConnect() {
        if (!clientId.trim() || !clientSecret.trim()) {
            setError("Both Client ID and Client Secret are required.");
            return;
        }
        setError(null);
        setStep("connecting");
        try {
            await invoke("cmd_gdrive_set_credentials", {
                clientId: clientId.trim(),
                clientSecret: clientSecret.trim(),
            });
            await invoke("cmd_gdrive_connect");
            setStep("done");
            setTimeout(onConnected, 1500);
        } catch (e: any) {
            setError(String(e));
            setStep("credentials");
        }
    }

    return (
        <div className="min-h-screen bg-telegram-bg flex items-center justify-center p-6">
            <motion.div
                initial={{ opacity: 0, y: 20 }}
                animate={{ opacity: 1, y: 0 }}
                className="w-full max-w-md"
            >
                {/* Card */}
                <div className="bg-telegram-surface border border-telegram-border rounded-2xl overflow-hidden shadow-2xl">
                    {/* Header */}
                    <div className="relative p-6 pb-4 border-b border-telegram-border bg-gradient-to-br from-[#4285F4]/10 via-telegram-surface to-telegram-surface">
                        <div className="flex items-center gap-3 mb-1">
                            {/* Google Drive logo colours */}
                            <div className="w-10 h-10 rounded-xl bg-white flex items-center justify-center shadow-sm">
                                <svg viewBox="0 0 87.3 78" className="w-6 h-6">
                                    <path d="m6.6 66.85 3.85 6.65c.8 1.4 1.95 2.5 3.3 3.3l13.75-23.8h-27.5c0 1.55.4 3.1 1.2 4.5z" fill="#0066da"/>
                                    <path d="m43.65 25-13.75-23.8c-1.35.8-2.5 1.9-3.3 3.3l-25.4 44a9.06 9.06 0 0 0 -1.2 4.5h27.5z" fill="#00ac47"/>
                                    <path d="m73.55 76.8c1.35-.8 2.5-1.9 3.3-3.3l1.6-2.75 7.65-13.25c.8-1.4 1.2-2.95 1.2-4.5h-27.502l5.852 11.5z" fill="#ea4335"/>
                                    <path d="m43.65 25 13.75-23.8c-1.35-.8-2.9-1.2-4.5-1.2h-18.5c-1.6 0-3.15.45-4.5 1.2z" fill="#00832d"/>
                                    <path d="m59.8 53h-32.3l-13.75 23.8c1.35.8 2.9 1.2 4.5 1.2h50.8c1.6 0 3.15-.45 4.5-1.2z" fill="#2684fc"/>
                                    <path d="m73.4 26.5-12.7-22c-.8-1.4-1.95-2.5-3.3-3.3l-13.75 23.8 16.15 28h27.45c0-1.55-.4-3.1-1.2-4.5z" fill="#ffba00"/>
                                </svg>
                            </div>
                            <div>
                                <h2 className="text-lg font-bold text-telegram-text">Connect Google Drive</h2>
                                <p className="text-xs text-telegram-subtext">Mirror your Drive into Telegram storage</p>
                            </div>
                        </div>
                    </div>

                    {/* Body */}
                    <div className="p-6">
                        <AnimatePresence mode="wait">
                            {step === "intro" && (
                                <motion.div key="intro" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
                                    <p className="text-sm text-telegram-subtext mb-4 leading-relaxed">
                                        OmniSync will watch your Google Drive and automatically mirror new and changed files
                                        into your Telegram storage in near real-time.
                                    </p>
                                    <ul className="space-y-2 mb-6">
                                        {[
                                            "Polls Google Drive every 10 seconds for changes",
                                            "New files are automatically backed up to Telegram",
                                            "Your Google credentials never leave your device",
                                            "Requires your own Google Cloud OAuth app",
                                        ].map((item) => (
                                            <li key={item} className="flex items-start gap-2 text-sm text-telegram-text">
                                                <CheckCircle2 className="w-4 h-4 text-green-400 mt-0.5 shrink-0" />
                                                {item}
                                            </li>
                                        ))}
                                    </ul>
                                    <button
                                        onClick={() => setStep("credentials")}
                                        className="w-full flex items-center justify-center gap-2 bg-[#4285F4] hover:bg-[#3367d6] text-white font-medium py-2.5 rounded-xl transition-colors"
                                    >
                                        Get Started <ChevronRight className="w-4 h-4" />
                                    </button>
                                    <button onClick={onSkip} className="w-full mt-2 text-sm text-telegram-subtext hover:text-telegram-text py-2 transition-colors">
                                        Skip for now
                                    </button>
                                </motion.div>
                            )}

                            {step === "credentials" && (
                                <motion.div key="creds" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} className="space-y-4">
                                    <div className="flex items-start gap-2 p-3 bg-amber-500/10 border border-amber-500/20 rounded-xl text-xs text-amber-300 leading-relaxed">
                                        <Info className="w-4 h-4 mt-0.5 shrink-0" />
                                        <span>
                                            You need a Google Cloud OAuth 2.0 client ID.{" "}
                                            <button
                                                onClick={() => setShowHelp(!showHelp)}
                                                className="underline hover:text-amber-200"
                                            >
                                                {showHelp ? "Hide" : "How?"}
                                            </button>
                                        </span>
                                    </div>

                                    <AnimatePresence>
                                        {showHelp && (
                                            <motion.ol
                                                initial={{ height: 0, opacity: 0 }}
                                                animate={{ height: "auto", opacity: 1 }}
                                                exit={{ height: 0, opacity: 0 }}
                                                className="overflow-hidden text-xs text-telegram-subtext space-y-1 list-decimal list-inside pl-1"
                                            >
                                                <li>Go to <a href="https://console.cloud.google.com" target="_blank" rel="noopener noreferrer" className="text-telegram-primary underline inline-flex items-center gap-0.5">console.cloud.google.com <ExternalLink className="w-3 h-3" /></a></li>
                                                <li>Create a project → Enable the <strong>Google Drive API</strong></li>
                                                <li>OAuth consent screen → External → Add your email as a test user</li>
                                                <li>Credentials → Create → OAuth Client ID → <strong>Desktop app</strong></li>
                                                <li>Copy the Client ID and Client Secret below</li>
                                                <li>Add <code className="bg-telegram-hover px-1 rounded">http://127.0.0.1</code> as an authorised redirect origin</li>
                                            </motion.ol>
                                        )}
                                    </AnimatePresence>

                                    <div className="space-y-3">
                                        <div>
                                            <label className="text-xs text-telegram-subtext block mb-1">Client ID</label>
                                            <input
                                                id="gdrive-client-id"
                                                type="text"
                                                value={clientId}
                                                onChange={e => setClientId(e.target.value)}
                                                placeholder="xxxxxxxxx.apps.googleusercontent.com"
                                                className="w-full bg-telegram-hover border border-telegram-border rounded-lg px-3 py-2 text-sm text-telegram-text placeholder:text-telegram-subtext/50 focus:outline-none focus:border-telegram-primary/50"
                                            />
                                        </div>
                                        <div>
                                            <label className="text-xs text-telegram-subtext block mb-1">Client Secret</label>
                                            <input
                                                id="gdrive-client-secret"
                                                type="password"
                                                value={clientSecret}
                                                onChange={e => setClientSecret(e.target.value)}
                                                placeholder="GOCSPX-…"
                                                className="w-full bg-telegram-hover border border-telegram-border rounded-lg px-3 py-2 text-sm text-telegram-text placeholder:text-telegram-subtext/50 focus:outline-none focus:border-telegram-primary/50"
                                            />
                                        </div>
                                    </div>

                                    {error && (
                                        <div className="flex items-start gap-2 p-3 bg-red-500/10 border border-red-500/20 rounded-xl text-xs text-red-400">
                                            <X className="w-4 h-4 mt-0.5 shrink-0" />
                                            {error}
                                        </div>
                                    )}

                                    <button
                                        id="gdrive-connect-btn"
                                        onClick={handleConnect}
                                        className="w-full flex items-center justify-center gap-2 bg-[#4285F4] hover:bg-[#3367d6] text-white font-medium py-2.5 rounded-xl transition-colors"
                                    >
                                        Sign in with Google
                                    </button>
                                    <button onClick={onSkip} className="w-full text-sm text-telegram-subtext hover:text-telegram-text py-1.5 transition-colors">
                                        Skip for now
                                    </button>
                                </motion.div>
                            )}

                            {step === "connecting" && (
                                <motion.div key="connecting" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} className="py-8 flex flex-col items-center gap-4">
                                    <Loader2 className="w-10 h-10 text-[#4285F4] animate-spin" />
                                    <p className="text-sm text-telegram-subtext text-center">
                                        A browser window has opened.<br />
                                        Please sign in and grant access to continue.
                                    </p>
                                </motion.div>
                            )}

                            {step === "done" && (
                                <motion.div key="done" initial={{ opacity: 0, scale: 0.9 }} animate={{ opacity: 1, scale: 1 }} className="py-8 flex flex-col items-center gap-3">
                                    <div className="w-14 h-14 rounded-full bg-green-500/20 flex items-center justify-center">
                                        <CheckCircle2 className="w-8 h-8 text-green-400" />
                                    </div>
                                    <p className="text-base font-semibold text-telegram-text">Google Drive Connected!</p>
                                    <p className="text-xs text-telegram-subtext">Sync starts in a moment…</p>
                                </motion.div>
                            )}
                        </AnimatePresence>
                    </div>
                </div>
            </motion.div>
        </div>
    );
}
