import { HardDrive, LayoutGrid, Sun, Moon, RefreshCw, CheckCircle2, AlertCircle, WifiOff } from 'lucide-react';
import { useTheme } from '../../context/ThemeContext';
import { useGDriveSync } from '../../hooks/useGDriveSync';
import { motion, AnimatePresence } from 'framer-motion';

interface TopBarProps {
    currentFolderName: string;
    selectedIds: number[];
    onShowMoveModal: () => void;
    onBulkDownload: () => void;
    onBulkDelete: () => void;
    onDownloadFolder: () => void;
    viewMode: 'grid' | 'list';
    setViewMode: (mode: 'grid' | 'list') => void;
    searchTerm: string;
    onSearchChange: (term: string) => void;
}

function SyncBadge() {
    const { status, recentEvents } = useGDriveSync();

    if (status.state === 'disconnected') return null;

    const icon = {
        connecting: <RefreshCw className="w-3.5 h-3.5 animate-spin" />,
        connected:  <CheckCircle2 className="w-3.5 h-3.5" />,
        syncing:    <RefreshCw className="w-3.5 h-3.5 animate-spin" />,
        error:      <AlertCircle className="w-3.5 h-3.5" />,
        disconnected: <WifiOff className="w-3.5 h-3.5" />,
    }[status.state];

    const color = {
        connecting:   'text-amber-400 bg-amber-400/10 border-amber-400/20',
        connected:    'text-green-400 bg-green-400/10 border-green-400/20',
        syncing:      'text-blue-400 bg-blue-400/10 border-blue-400/20',
        error:        'text-red-400 bg-red-400/10 border-red-400/20',
        disconnected: 'text-telegram-subtext bg-telegram-hover border-telegram-border',
    }[status.state];

    const lastEvent = recentEvents[0];

    return (
        <div className="relative group">
            <div className={`flex items-center gap-1.5 px-2.5 py-1 rounded-full border text-xs font-medium transition-all ${color}`}>
                {icon}
                <span className="hidden sm:inline">
                    {status.state === 'syncing' ? 'Syncing…' :
                     status.state === 'connected' ? 'Drive synced' :
                     status.state === 'connecting' ? 'Connecting…' :
                     status.state === 'error' ? 'Sync error' : ''}
                </span>
                {status.filesSynced > 0 && (
                    <span className="ml-0.5 opacity-60">{status.filesSynced}</span>
                )}
            </div>

            {/* Tooltip */}
            <div className="absolute right-0 top-full mt-2 w-64 bg-telegram-surface border border-telegram-border rounded-xl shadow-2xl p-3 opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none z-50">
                <div className="flex items-center justify-between mb-2">
                    <span className="text-xs font-semibold text-telegram-text">Google Drive Sync</span>
                    {status.lastSynced && (
                        <span className="text-[10px] text-telegram-subtext">Last: {status.lastSynced}</span>
                    )}
                </div>
                <p className="text-[11px] text-telegram-subtext mb-2">{status.message}</p>
                {lastEvent && (
                    <div className="text-[10px] text-telegram-subtext border-t border-telegram-border pt-2">
                        <span className="text-telegram-primary font-medium">Latest: </span>
                        {lastEvent.message}
                    </div>
                )}
                <div className="mt-2 text-[10px] text-telegram-subtext">
                    {status.filesSynced} file{status.filesSynced !== 1 ? 's' : ''} synced this session
                </div>
            </div>
        </div>
    );
}

export function TopBar({
    currentFolderName, selectedIds, onShowMoveModal, onBulkDownload, onBulkDelete,
    onDownloadFolder, viewMode, setViewMode, searchTerm, onSearchChange
}: TopBarProps) {
    const { theme, toggleTheme } = useTheme();

    return (
        <header className="h-14 border-b border-telegram-border flex items-center px-4 justify-between bg-telegram-surface/80 backdrop-blur-md sticky top-0 z-10" onClick={e => e.stopPropagation()}>
            <div className="flex items-center gap-4">
                <div className="flex items-center text-sm breadcrumbs text-telegram-subtext select-none">
                    <span className="hover:text-telegram-text cursor-pointer transition-colors">Start</span>
                    <span className="mx-2">/</span>
                    <span className="text-telegram-text font-medium">{currentFolderName}</span>
                </div>
            </div>

            <div className="flex-1 max-w-md mx-4">
                <input
                    type="text"
                    placeholder="Search files..."
                    className="w-full bg-telegram-hover border border-telegram-border rounded-lg px-3 py-1.5 text-sm text-telegram-text placeholder:text-telegram-subtext focus:outline-none focus:border-telegram-primary/50 transition-colors"
                    value={searchTerm}
                    onChange={(e) => onSearchChange(e.target.value)}
                />
            </div>

            <div className="flex items-center gap-2">
                <AnimatePresence>
                    {selectedIds.length > 0 && (
                        <motion.div
                            initial={{ opacity: 0, x: 10 }}
                            animate={{ opacity: 1, x: 0 }}
                            exit={{ opacity: 0, x: 10 }}
                            className="flex items-center gap-2 mr-4"
                        >
                            <span className="text-xs text-telegram-subtext mr-2">{selectedIds.length} Selected</span>
                            <button onClick={onShowMoveModal} className="px-3 py-1.5 bg-telegram-primary/20 hover:bg-telegram-primary/30 text-telegram-primary rounded-md text-xs transition font-medium">Move to...</button>
                            <button onClick={onBulkDownload} className="px-3 py-1.5 bg-telegram-hover hover:bg-telegram-border rounded-md text-xs text-telegram-text transition">Download Selected</button>
                            <button onClick={onBulkDelete} className="px-3 py-1.5 bg-red-500/10 hover:bg-red-500/20 text-red-400 rounded-md text-xs transition">Delete</button>
                        </motion.div>
                    )}
                </AnimatePresence>

                {/* Google Drive sync badge */}
                <SyncBadge />

                <button onClick={onDownloadFolder} className="p-2 hover:bg-telegram-hover rounded-md text-telegram-subtext hover:text-telegram-text transition group relative" title="Download Folder">
                    <HardDrive className="w-5 h-5" />
                    <span className="absolute -bottom-8 left-1/2 -translate-x-1/2 text-[10px] bg-telegram-surface border border-telegram-border px-2 py-1 rounded opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none whitespace-nowrap z-50 shadow-lg">
                        Download All Files
                    </span>
                </button>

                <button
                    onClick={() => setViewMode(viewMode === 'grid' ? 'list' : 'grid')}
                    className="p-2 hover:bg-telegram-hover rounded-md text-telegram-subtext hover:text-telegram-text transition relative group"
                    title="Toggle Layout"
                >
                    <LayoutGrid className="w-5 h-5" />
                    <span className="absolute -bottom-8 left-1/2 -translate-x-1/2 text-[10px] bg-telegram-surface border border-telegram-border px-2 py-1 rounded opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none whitespace-nowrap z-50 shadow-lg">
                        {viewMode === 'grid' ? 'Switch to List' : 'Switch to Grid'}
                    </span>
                </button>

                <div className="w-px h-6 bg-telegram-border mx-1"></div>

                <button
                    onClick={toggleTheme}
                    className="p-2 hover:bg-telegram-hover rounded-md text-telegram-subtext hover:text-telegram-text transition relative group"
                    title={theme === 'dark' ? 'Switch to Light Mode' : 'Switch to Dark Mode'}
                >
                    {theme === 'dark' ? <Sun className="w-5 h-5" /> : <Moon className="w-5 h-5" />}
                    <span className="absolute -bottom-8 left-1/2 -translate-x-1/2 text-[10px] bg-telegram-surface border border-telegram-border px-2 py-1 rounded opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none whitespace-nowrap z-50 shadow-lg">
                        {theme === 'dark' ? 'Light Mode' : 'Dark Mode'}
                    </span>
                </button>
            </div>
        </header>
    );
}
