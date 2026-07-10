/**
 * QuizBatchPanel.tsx
 *
 * "Take more quizzes" section — for users who want to actively help train
 * the model (v1.2 #3, ROADMAP.md). Batch of up to 10 alias-confirm
 * questions at a time; answering one immediately reveals the next.
 */

import { useState } from "react";
import { Sparkles } from "lucide-react";
import { useDailyQuiz, useSubmitQuizAnswer } from "@/hooks/useTauri";
import { QuizCard } from "./QuizCard";

const BATCH_SIZE = 10;

export function QuizBatchPanel() {
  const { data: questions, isLoading, refetch } = useDailyQuiz(BATCH_SIZE);
  const submitAnswer = useSubmitQuizAnswer();
  const [resolved, setResolved] = useState<Set<string>>(new Set());

  const remaining = (questions ?? []).filter((q) => !resolved.has(q.id));

  const handleAnswer = (questionId: string, kind: string, entityIdA: string, entityIdB: string, confirmed: boolean) => {
    submitAnswer.mutate({ questionId, kind, entityIdA, entityIdB, confirmed });
    setResolved((prev) => new Set(prev).add(questionId));
  };

  const handleSkip = (questionId: string) => {
    setResolved((prev) => new Set(prev).add(questionId));
  };

  if (isLoading) {
    return <p className="text-sm text-text-tertiary">Loading quizzes…</p>;
  }

  if (remaining.length === 0) {
    return (
      <div className="rounded-xl border border-border-primary bg-bg-secondary p-8 text-center space-y-3">
        <Sparkles size={28} className="mx-auto text-accent-primary" />
        <p className="text-sm text-text-secondary">
          No more quizzes right now — Cortex is confident about your entity
          relationships. Check back after indexing more documents.
        </p>
        <button
          onClick={() => void refetch()}
          className="text-xs text-accent-primary hover:underline"
        >
          Refresh
        </button>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <p className="text-sm text-text-secondary">
        {remaining.length} question{remaining.length === 1 ? "" : "s"} —
        answering helps Cortex merge duplicate entities correctly.
      </p>
      {remaining.map((q) => (
        <QuizCard
          key={q.id}
          question={q}
          onAnswer={(confirmed) =>
            handleAnswer(q.id, q.kind, q.entityIdA, q.entityIdB, confirmed)
          }
          onSkip={() => handleSkip(q.id)}
          pending={submitAnswer.isPending}
        />
      ))}
    </div>
  );
}
