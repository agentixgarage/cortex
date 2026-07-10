/**
 * AboutYouStep.tsx
 *
 * Onboarding "About You" step (v1.2 #2, ROADMAP.md). Optional, skippable.
 * Collects: display name, aliases, family members, countries, currencies.
 *
 * This seeds ChatEngine's RAG system prompt (see
 * src-tauri/src/chat/engine.rs::build_system_prompt) with context documents
 * alone cannot supply — fixes cases like "generate my family tree" where the
 * LLM correctly refuses to guess relationships from shared surnames alone.
 *
 * All fields optional. User can fill in some, none, or all — partial profiles
 * are valid and still improve retrieval/answers incrementally.
 */

import { useState } from "react";
import { Plus, X, ArrowRight, SkipForward, User2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { useSaveUserProfile } from "@/hooks/useTauri";
import type { FamilyMember } from "@/lib/types";

export interface AboutYouStepProps {
  onContinue: () => void;
  onSkip: () => void;
}

function ChipInput({
  label,
  placeholder,
  values,
  onChange,
}: {
  label: string;
  placeholder: string;
  values: string[];
  onChange: (next: string[]) => void;
}) {
  const [draft, setDraft] = useState("");

  const commit = () => {
    const v = draft.trim();
    if (v && !values.includes(v)) {
      onChange([...values, v]);
    }
    setDraft("");
  };

  return (
    <div className="space-y-2">
      <label className="text-sm font-medium text-text-primary">{label}</label>
      <div className="flex flex-wrap gap-2">
        {values.map((v) => (
          <span
            key={v}
            className="flex items-center gap-1 rounded-full bg-bg-tertiary px-3 py-1 text-xs text-text-primary"
          >
            {v}
            <X
              size={12}
              className="cursor-pointer text-text-tertiary hover:text-text-primary"
              onClick={() => onChange(values.filter((x) => x !== v))}
            />
          </span>
        ))}
      </div>
      <div className="flex gap-2">
        <input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              commit();
            }
          }}
          placeholder={placeholder}
          className="flex-1 rounded-lg border border-border-primary bg-bg-secondary px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent-primary focus:outline-none"
        />
        <button
          type="button"
          onClick={commit}
          className="rounded-lg border border-border-primary px-3 py-2 text-text-secondary hover:bg-bg-tertiary"
        >
          <Plus size={16} />
        </button>
      </div>
    </div>
  );
}

export function AboutYouStep({ onContinue, onSkip }: AboutYouStepProps) {
  const [displayName, setDisplayName] = useState("");
  const [aliases, setAliases] = useState<string[]>([]);
  const [familyMembers, setFamilyMembers] = useState<FamilyMember[]>([]);
  const [familyName, setFamilyName] = useState("");
  const [familyRelation, setFamilyRelation] = useState("");
  const [countries, setCountries] = useState<string[]>([]);
  const [currencies, setCurrencies] = useState<string[]>([]);
  const saveProfile = useSaveUserProfile();

  const addFamilyMember = () => {
    const name = familyName.trim();
    const relation = familyRelation.trim();
    if (name && relation) {
      setFamilyMembers((prev) => [...prev, { name, relation }]);
      setFamilyName("");
      setFamilyRelation("");
    }
  };

  const removeFamilyMember = (idx: number) => {
    setFamilyMembers((prev) => prev.filter((_, i) => i !== idx));
  };

  const hasAnyInput =
    displayName.trim() !== "" ||
    aliases.length > 0 ||
    familyMembers.length > 0 ||
    countries.length > 0 ||
    currencies.length > 0;

  const handleContinue = async () => {
    if (hasAnyInput) {
      try {
        await saveProfile.mutateAsync({
          displayName: displayName.trim(),
          aliases,
          familyMembers,
          countries,
          currencies,
        });
      } catch (e) {
        console.error("save_user_profile failed", e);
      }
    }
    onContinue();
  };

  return (
    <div className="space-y-6 animate-in fade-in duration-500">
      <div className="text-center space-y-2">
        <div className="mx-auto flex h-14 w-14 items-center justify-center rounded-2xl bg-accent-primary/10">
          <User2 size={28} className="text-accent-primary" />
        </div>
        <h2 className="section-title text-text-primary">About you (optional)</h2>
        <p className="text-sm text-text-secondary max-w-md mx-auto">
          Cortex answers questions better when it knows a bit about you. This
          stays entirely on your device and can be edited any time in
          Settings.
        </p>
      </div>

      <div className="space-y-5 max-h-[50vh] overflow-y-auto px-1">
        <div className="space-y-2">
          <label className="text-sm font-medium text-text-primary">
            Your name
          </label>
          <input
            value={displayName}
            onChange={(e) => setDisplayName(e.target.value)}
            placeholder="e.g. Alex Doe"
            className="w-full rounded-lg border border-border-primary bg-bg-secondary px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent-primary focus:outline-none"
          />
        </div>

        <ChipInput
          label="Also known as (nicknames, legal-name variants)"
          placeholder="e.g. A. Doe"
          values={aliases}
          onChange={setAliases}
        />

        <div className="space-y-2">
          <label className="text-sm font-medium text-text-primary">
            Family members
          </label>
          <div className="flex flex-wrap gap-2">
            {familyMembers.map((f, i) => (
              <span
                key={`${f.name}-${i}`}
                className="flex items-center gap-1 rounded-full bg-bg-tertiary px-3 py-1 text-xs text-text-primary"
              >
                {f.name} ({f.relation})
                <X
                  size={12}
                  className="cursor-pointer text-text-tertiary hover:text-text-primary"
                  onClick={() => removeFamilyMember(i)}
                />
              </span>
            ))}
          </div>
          <div className="flex gap-2">
            <input
              value={familyName}
              onChange={(e) => setFamilyName(e.target.value)}
              placeholder="Name"
              className="flex-1 rounded-lg border border-border-primary bg-bg-secondary px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent-primary focus:outline-none"
            />
            <input
              value={familyRelation}
              onChange={(e) => setFamilyRelation(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  addFamilyMember();
                }
              }}
              placeholder="Relation (spouse, child…)"
              className="flex-1 rounded-lg border border-border-primary bg-bg-secondary px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent-primary focus:outline-none"
            />
            <button
              type="button"
              onClick={addFamilyMember}
              className="rounded-lg border border-border-primary px-3 py-2 text-text-secondary hover:bg-bg-tertiary"
            >
              <Plus size={16} />
            </button>
          </div>
        </div>

        <ChipInput
          label="Countries you have documents or assets in"
          placeholder="e.g. India"
          values={countries}
          onChange={setCountries}
        />

        <ChipInput
          label="Currencies your documents commonly use"
          placeholder="e.g. INR, USD"
          values={currencies}
          onChange={setCurrencies}
        />
      </div>

      <div className="flex items-center justify-between pt-2">
        <button
          onClick={onSkip}
          className={cn(
            "inline-flex items-center gap-2 rounded-lg px-4 py-2 text-sm font-medium text-text-secondary transition-colors hover:bg-bg-tertiary",
          )}
        >
          <SkipForward size={16} />
          Skip for now
        </button>
        <button
          onClick={() => void handleContinue()}
          disabled={saveProfile.isPending}
          className="inline-flex items-center gap-2 rounded-lg bg-accent-primary px-6 py-3 text-sm font-medium text-white transition-colors hover:bg-accent-hover disabled:opacity-50"
        >
          Continue
          <ArrowRight size={16} />
        </button>
      </div>
    </div>
  );
}
