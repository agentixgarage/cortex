/**
 * OnboardingPage - 6-step first-time user wizard.
 *
 * Steps:
 *  0. Welcome       - branding and value prop
 *  1. Connect AI    - (D-12) 2x2 provider grid with inline forms
 *  2. About You     - v1.2 #2 (ROADMAP.md), optional/skippable profile seed
 *  3. Select Folders - pick folders to watch
 *  4. Scanning Progress - live indexing feedback
 *  5. Spaces Ready  - discovered spaces and CTA
 */

import { useState, useCallback, useEffect } from "react";
import { useNavigate } from "react-router-dom";
import {
  Brain,
  FolderOpen,
  Plus,
  X,
  Loader2,
  ArrowRight,
  Check,
  SkipForward,
  Sparkles,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useOnboardingStore, useIndexingStore } from "@/lib/stores";
import {
  useAddWatchedFolder,
  useTriggerScan,
  useSpaces,
} from "@/hooks/useTauri";
import { ConnectAiStep } from "@/components/ai/ConnectAiStep";
import { AboutYouStep } from "@/components/onboarding/AboutYouStep";
import { isTauri } from "@/lib/tauri";
import { resolveIcon } from "@/lib/icons";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

// Default suggested folders
const SUGGESTED_FOLDERS = [
  { path: "~/Documents", label: "Documents" },
  { path: "~/Desktop", label: "Desktop" },
  { path: "~/Downloads", label: "Downloads" },
];

// Step indicator
function StepIndicator({ current, total }: { current: number; total: number }) {
  return (
    <div className="flex items-center gap-2">
      {Array.from({ length: total }).map((_, i) => (
        <div
          key={i}
          className={cn(
            "h-2 rounded-full transition-all duration-300",
            i === current
              ? "w-8 bg-accent-primary"
              : i < current
                ? "w-2 bg-accent-primary/60"
                : "w-2 bg-bg-tertiary",
          )}
        />
      ))}
    </div>
  );
}

export default function OnboardingPage() {
  const [step, setStep] = useState(0);
  const [selectedFolders, setSelectedFolders] = useState<string[]>([]);
  const [customFolder, setCustomFolder] = useState("");
  const [scanningProgress, setScanningProgress] = useState({
    processed: 0,
    total: 0,
    currentFile: "",
  });
  const navigate = useNavigate();
  const { setCompleted } = useOnboardingStore();
  const indexingStore = useIndexingStore();
  const addWatchedFolder = useAddWatchedFolder();
  const triggerScan = useTriggerScan();
  const { data: spaces } = useSpaces();

  // Toggle a suggested folder
  const toggleFolder = (path: string) => {
    setSelectedFolders((prev) =>
      prev.includes(path) ? prev.filter((p) => p !== path) : [...prev, path],
    );
  };

  // Add custom folder — in Tauri opens native picker; in browser uses text input
  const addCustomFolder = useCallback(async () => {
    if (isTauri()) {
      try {
        const selected = await openDialog({ directory: true, multiple: false });
        if (selected) {
          const folderPath = typeof selected === "string" ? selected : selected[0];
          if (folderPath && !selectedFolders.includes(folderPath)) {
            setSelectedFolders((prev) => [...prev, folderPath]);
          }
        }
        return;
      } catch {
        // Picker failed unexpectedly — fall through to text-input behavior
      }
    }
    if (customFolder.trim() && !selectedFolders.includes(customFolder.trim())) {
      setSelectedFolders((prev) => [...prev, customFolder.trim()]);
      setCustomFolder("");
    }
  }, [customFolder, selectedFolders]);

  // Remove a folder from selection
  const removeFolder = (path: string) => {
    setSelectedFolders((prev) => prev.filter((p) => p !== path));
  };

  // Start scanning selected folders
  const startScanning = async () => {
    setStep(4);  // Step 4 is now Scanning
    let totalProcessed = 0;
    const totalFolders = selectedFolders.length;
    setScanningProgress({ processed: 0, total: totalFolders * 100, currentFile: "Starting..." });

    for (const folderPath of selectedFolders) {
      try {
        const result = await addWatchedFolder.mutateAsync(folderPath);
        setScanningProgress((prev) => ({
          ...prev,
          currentFile: `Scanning ${folderPath}...`,
        }));
        await triggerScan.mutateAsync(result.id);
        totalProcessed++;
        setScanningProgress({
          processed: totalProcessed * 100,
          total: totalFolders * 100,
          currentFile: `Completed ${folderPath}`,
        });
      } catch {
        totalProcessed++;
        setScanningProgress((prev) => ({
          processed: totalProcessed * 100,
          total: totalFolders * 100,
          currentFile: `Skipped ${folderPath}`,
        }));
      }
    }

    // Auto-advance to step 5 (Spaces Ready)
    setTimeout(() => setStep(5), 1000);
  };

  // Listen for indexing events and update progress display
  useEffect(() => {
    if (step === 4 && indexingStore.isIndexing) {  // step 4 is now Scanning
      setScanningProgress({
        processed: indexingStore.filesProcessed,
        total: indexingStore.totalFiles,
        currentFile: indexingStore.currentFile,
      });
    }
  }, [step, indexingStore.isIndexing, indexingStore.filesProcessed, indexingStore.totalFiles, indexingStore.currentFile]);

  // Complete onboarding
  const completeOnboarding = () => {
    setCompleted(true);
    navigate("/spaces");
  };

  const progressPercent =
    scanningProgress.total > 0
      ? Math.round((scanningProgress.processed / scanningProgress.total) * 100)
      : 0;

  return (
    <div className="flex min-h-[80vh] items-center justify-center">
      <div className="w-full max-w-lg space-y-8">
        {/* Step indicator */}
        <div className="flex justify-center">
          <StepIndicator current={step} total={6} />
        </div>

        {/* Step 0: Welcome */}
        {step === 0 && (
          <div className="space-y-6 text-center animate-in fade-in duration-500">
            <div className="mx-auto flex h-20 w-20 items-center justify-center rounded-2xl bg-accent-primary/10">
              <Brain size={40} className="text-accent-primary" />
            </div>
            <div className="space-y-3">
              <h1 className="page-title text-text-primary">
                Welcome to Cortex
              </h1>
              <p className="text-lg font-medium text-accent-primary">
                Find anything. Organize nothing.
              </p>
              <p className="text-text-secondary max-w-md mx-auto">
                Cortex watches your document folders and automatically organizes
                everything using AI. Drop your folders, and let smart spaces
                emerge.
              </p>
            </div>
            <button
              onClick={() => setStep(1)}
              className="inline-flex items-center gap-2 rounded-lg bg-accent-primary px-6 py-3 text-sm font-medium text-white transition-colors hover:bg-accent-hover"
            >
              Get Started
              <ArrowRight size={16} />
            </button>
          </div>
        )}

        {/* Step 1: Connect AI (NEW — D-12) */}
        {step === 1 && (
          <ConnectAiStep
            onContinue={() => setStep(2)}
            onSkip={() => setStep(2)}
          />
        )}

        {/* Step 2: About You (v1.2 #2 — ROADMAP.md, optional/skippable) */}
        {step === 2 && (
          <AboutYouStep
            onContinue={() => setStep(3)}
            onSkip={() => setStep(3)}
          />
        )}

        {/* Step 3: Select Folders (was step 1) */}
        {step === 3 && (
          <div className="space-y-6 animate-in fade-in duration-500">
            <div className="text-center space-y-2">
              <h2 className="section-title text-text-primary">
                Choose folders to watch
              </h2>
              <p className="text-sm text-text-secondary">
                Cortex will monitor these folders for documents
              </p>
            </div>

            {/* Suggested folders */}
            <div className="space-y-2">
              {SUGGESTED_FOLDERS.map((folder) => (
                <button
                  key={folder.path}
                  onClick={() => toggleFolder(folder.path)}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-lg border px-4 py-3 text-left transition-all",
                    selectedFolders.includes(folder.path)
                      ? "border-accent-primary bg-accent-primary/5 text-text-primary"
                      : "border-border-primary bg-bg-secondary text-text-secondary hover:border-border-secondary hover:bg-bg-tertiary",
                  )}
                >
                  <FolderOpen size={20} />
                  <span className="flex-1 text-sm font-medium">
                    {folder.label}
                  </span>
                  <span className="text-xs text-text-tertiary font-mono">
                    {folder.path}
                  </span>
                  {selectedFolders.includes(folder.path) && (
                    <Check size={16} className="text-accent-primary" />
                  )}
                </button>
              ))}
            </div>

            {/* Custom folder input */}
            <div className="flex gap-2">
              <input
                type="text"
                value={customFolder}
                onChange={(e) => setCustomFolder(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && addCustomFolder()}
                placeholder="Add custom folder path..."
                className="flex-1 rounded-lg border border-border-primary bg-bg-secondary px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent-primary focus:outline-none"
              />
              <button
                onClick={addCustomFolder}
                className="inline-flex items-center gap-1 rounded-lg border border-border-primary px-3 py-2 text-sm text-text-secondary hover:bg-bg-tertiary transition-colors"
              >
                <Plus size={16} />
                Add
              </button>
            </div>

            {/* Selected custom folders list */}
            {selectedFolders.filter(
              (f) => !SUGGESTED_FOLDERS.some((s) => s.path === f),
            ).length > 0 && (
              <div className="space-y-1">
                <p className="text-xs font-medium text-text-tertiary uppercase tracking-wider">
                  Custom Folders
                </p>
                {selectedFolders
                  .filter((f) => !SUGGESTED_FOLDERS.some((s) => s.path === f))
                  .map((folder) => (
                    <div
                      key={folder}
                      className="flex items-center gap-2 rounded-md bg-bg-secondary px-3 py-2 text-sm"
                    >
                      <FolderOpen size={14} className="text-text-tertiary" />
                      <span className="flex-1 font-mono text-text-secondary truncate">
                        {folder}
                      </span>
                      <button
                        onClick={() => removeFolder(folder)}
                        className="text-text-tertiary hover:text-status-error transition-colors"
                      >
                        <X size={14} />
                      </button>
                    </div>
                  ))}
              </div>
            )}

            {/* Action button */}
            <div className="flex justify-between pt-2">
              <button
                onClick={() => setStep(2)}
                className="text-sm text-text-tertiary hover:text-text-secondary transition-colors"
              >
                Back
              </button>
              <button
                onClick={startScanning}
                disabled={selectedFolders.length === 0}
                className={cn(
                  "inline-flex items-center gap-2 rounded-lg px-6 py-3 text-sm font-medium transition-colors",
                  selectedFolders.length > 0
                    ? "bg-accent-primary text-white hover:bg-accent-hover"
                    : "bg-bg-tertiary text-text-tertiary cursor-not-allowed",
                )}
              >
                Start Scanning
                <ArrowRight size={16} />
              </button>
            </div>
          </div>
        )}

        {/* Step 4: Scanning Progress (was step 2) */}
        {step === 4 && (
          <div className="space-y-6 text-center animate-in fade-in duration-500">
            <div className="mx-auto flex h-16 w-16 items-center justify-center rounded-2xl bg-accent-primary/10">
              <Loader2
                size={32}
                className="text-accent-primary animate-spin"
              />
            </div>
            <div className="space-y-2">
              <h2 className="section-title text-text-primary">
                Discovering your documents...
              </h2>
              <p className="text-sm text-text-secondary">
                Cortex is scanning your folders and building a knowledge graph
              </p>
            </div>

            {/* Progress bar */}
            <div className="space-y-2">
              <div className="h-2 w-full rounded-full bg-bg-tertiary overflow-hidden">
                <div
                  className="h-full rounded-full bg-accent-primary transition-all duration-500"
                  style={{ width: `${progressPercent}%` }}
                />
              </div>
              <div className="flex justify-between text-xs text-text-tertiary">
                <span>{progressPercent}% complete</span>
                <span>
                  {scanningProgress.processed} / {scanningProgress.total} files
                </span>
              </div>
            </div>

            {/* Current file */}
            <div className="rounded-md bg-bg-secondary px-4 py-2">
              <p className="text-xs font-mono text-text-tertiary truncate">
                {scanningProgress.currentFile || "Preparing..."}
              </p>
            </div>

            {/* Skip button */}
            <button
              onClick={() => setStep(5)}
              className="inline-flex items-center gap-1 text-sm text-text-tertiary hover:text-text-secondary transition-colors"
            >
              <SkipForward size={14} />
              Skip
            </button>
          </div>
        )}

        {/* Step 5: Spaces Ready (was step 3) */}
        {step === 5 && (
          <div className="space-y-6 text-center animate-in fade-in duration-500">
            <div className="mx-auto flex h-16 w-16 items-center justify-center rounded-2xl bg-status-success/10">
              <Sparkles size={32} className="text-status-success" />
            </div>
            <div className="space-y-2">
              <h2 className="section-title text-text-primary">
                Your Smart Spaces are ready!
              </h2>
              <p className="text-sm text-text-secondary">
                Cortex has organized your documents into intelligent categories
              </p>
            </div>

            {/* Discovered spaces grid */}
            {spaces && spaces.length > 0 ? (
              <div className="grid grid-cols-2 gap-3 text-left">
                {spaces.slice(0, 6).map((space) => {
                  const IconComponent = resolveIcon(space.icon);
                  return (
                    <div
                      key={space.id}
                      className="rounded-lg border border-border-primary bg-bg-secondary p-3 space-y-1"
                    >
                      <div className="flex items-center gap-2">
                        <div
                          className="flex h-8 w-8 items-center justify-center rounded-md"
                          style={{ backgroundColor: space.color + "20" }}
                        >
                          <IconComponent
                            size={16}
                            style={{ color: space.color }}
                          />
                        </div>
                        <div className="flex-1 min-w-0">
                          <p className="text-sm font-medium text-text-primary truncate">
                            {space.name}
                          </p>
                          <p className="text-xs text-text-tertiary">
                            {space.documentCount} docs
                          </p>
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            ) : (
              <div className="rounded-lg border border-border-primary bg-bg-secondary p-6">
                <p className="text-sm text-text-tertiary">
                  Spaces will appear as documents are indexed
                </p>
              </div>
            )}

            {/* CTA */}
            <button
              onClick={completeOnboarding}
              className="inline-flex items-center gap-2 rounded-lg bg-accent-primary px-6 py-3 text-sm font-medium text-white transition-colors hover:bg-accent-hover"
            >
              Explore Your Spaces
              <ArrowRight size={16} />
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
