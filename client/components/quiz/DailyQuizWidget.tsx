/**
 * DailyQuizWidget.tsx
 *
 * Dashboard card: one quiz question per day, non-intrusive. Renders nothing
 * when there are no candidates (most days, for small corpora). Answering or
 * skipping calls submit_quiz_answer / just hides locally (skip does not log
 * — the user can be re-asked the same question later if they change their
 * mind; only explicit Yes/No answers are logged as feedback signal).
 */

import { useState } from "react";
import { Link } from "react-router-dom";
import { useDailyQuiz, useSubmitQuizAnswer } from "@/hooks/useTauri";
import { QuizCard } from "./QuizCard";

export function DailyQuizWidget() {
  const { data: questions, isLoading } = useDailyQuiz(1);
  const submitAnswer = useSubmitQuizAnswer();
  const [skipped, setSkipped] = useState<Set<string>>(new Set());

  if (isLoading) return null;

  const question = (questions ?? []).find((q) => !skipped.has(q.id));
  if (!question) return null;

  const handleAnswer = (confirmed: boolean) => {
    submitAnswer.mutate({
      questionId: question.id,
      kind: question.kind,
      entityIdA: question.entityIdA,
      entityIdB: question.entityIdB,
      confirmed,
    });
  };

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-text-secondary">
          Quick question
        </h3>
        <Link
          to="/insights#quizzes"
          className="text-xs text-accent-primary hover:underline"
        >
          Take more quizzes
        </Link>
      </div>
      <QuizCard
        question={question}
        onAnswer={handleAnswer}
        onSkip={() => setSkipped((prev) => new Set(prev).add(question.id))}
        pending={submitAnswer.isPending}
      />
    </div>
  );
}
