/**
 * QuizCard.tsx
 *
 * Renders a single alias-confirm quiz question with Yes/No/Skip. Reusable —
 * used by DailyQuizWidget (Dashboard, 1/day) and the "Take more quizzes" tab
 * under Insights (batch mode). Also designed to be dropped into onboarding's
 * Scanning step in a future pass, once entities start appearing mid-scan
 * (first-ever scan has zero candidates, since no entities exist yet).
 */

import { HelpCircle, Check, X, SkipForward } from "lucide-react";
import type { QuizQuestion } from "@/lib/types";

export interface QuizCardProps {
  question: QuizQuestion;
  onAnswer: (confirmed: boolean) => void;
  onSkip: () => void;
  pending?: boolean;
}

export function QuizCard({ question, onAnswer, onSkip, pending }: QuizCardProps) {
  return (
    <div className="rounded-xl border border-border-primary bg-bg-secondary p-4 space-y-3">
      <div className="flex items-start gap-2">
        <HelpCircle size={18} className="mt-0.5 shrink-0 text-accent-primary" />
        <p className="text-sm text-text-primary">
          Is <span className="font-semibold">{question.nameA}</span> the same{" "}
          {question.entityType} as{" "}
          <span className="font-semibold">{question.nameB}</span>?
        </p>
      </div>
      <div className="flex gap-2">
        <button
          onClick={() => onAnswer(true)}
          disabled={pending}
          className="inline-flex items-center gap-1.5 rounded-lg bg-status-success/10 px-3 py-1.5 text-xs font-medium text-status-success transition-colors hover:bg-status-success/20 disabled:opacity-50"
        >
          <Check size={14} />
          Yes, same
        </button>
        <button
          onClick={() => onAnswer(false)}
          disabled={pending}
          className="inline-flex items-center gap-1.5 rounded-lg bg-status-error/10 px-3 py-1.5 text-xs font-medium text-status-error transition-colors hover:bg-status-error/20 disabled:opacity-50"
        >
          <X size={14} />
          No, different
        </button>
        <button
          onClick={onSkip}
          disabled={pending}
          className="inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-medium text-text-tertiary transition-colors hover:bg-bg-tertiary disabled:opacity-50"
        >
          <SkipForward size={14} />
          Skip
        </button>
      </div>
    </div>
  );
}
