import { Link, useNavigate } from "react-router-dom";
import {
  FileText,
  Brain,
  Clock,
  HardDrive,
  Search,
  FolderPlus,
} from "lucide-react";
import { useStats, useSpaces, useActivityFeed, useRecentDocuments } from "@/hooks/useTauri";
import { DailyQuizWidget } from "@/components/quiz/DailyQuizWidget";
import { formatBytes, safeDistance } from "@/lib/utils";

// ---------------------------------------------------------------------------
// Skeleton placeholder for loading states
// ---------------------------------------------------------------------------
function Skeleton({ className = "" }: { className?: string }) {
  return (
    <div
      className={`animate-pulse rounded-md bg-bg-tertiary ${className}`}
    />
  );
}

// ---------------------------------------------------------------------------
// Dashboard page — wired to live backend data via React Query hooks
// ---------------------------------------------------------------------------
export default function Dashboard() {
  const navigate = useNavigate();

  // Live data hooks
  const { data: stats, isLoading: statsLoading } = useStats();
  const { data: spaces, isLoading: spacesLoading } = useSpaces();
  const { data: activityItems, isLoading: activityLoading } = useActivityFeed();
  const { data: recentDocs, isLoading: recentLoading } = useRecentDocuments(8);

  // Derived: top 5 spaces sorted by doc count
  const topSpaces = spaces
    ? [...spaces].sort((a, b) => b.documentCount - a.documentCount).slice(0, 5)
    : [];

  // Stats grid config — values derived from live data
  const statCards = [
    {
      label: "Total Documents",
      value: stats ? stats.totalDocuments.toLocaleString() : "--",
      icon: FileText,
      color: "bg-blue-500/10 text-blue-500",
    },
    {
      label: "Smart Spaces",
      value: stats ? String(stats.smartSpaces) : "--",
      icon: Brain,
      color: "bg-purple-500/10 text-purple-500",
    },
    {
      label: "Last Scan",
      value: stats ? safeDistance(stats.lastScan) : "--",
      icon: Clock,
      color: "bg-green-500/10 text-green-500",
    },
    {
      label: "Index Size",
      value: stats ? formatBytes(stats.indexSize) : "--",
      icon: HardDrive,
      color: "bg-amber-500/10 text-amber-500",
    },
  ];

  return (
    <div className="space-y-8">
      {/* Greeting */}
      <div className="space-y-2">
        <h1 className="page-title text-text-primary">Welcome to Cortex.</h1>
        <p className="text-text-secondary">
          Here's what's happening with your documents.
        </p>
      </div>

      {/* Search Bar — navigates to /search on focus */}
      <div className="rounded-lg border border-border-primary bg-bg-secondary p-4">
        <button
          onClick={() => navigate("/search")}
          className="input-base w-full text-left text-text-tertiary flex items-center gap-2"
        >
          <Search size={16} />
          <span>Search your documents... (Cmd+K)</span>
        </button>
      </div>

      {/* Daily quiz (v1.2 #3) — renders nothing when no candidates exist */}
      <DailyQuizWidget />

      {/* Stats Grid */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        {statsLoading
          ? Array.from({ length: 4 }).map((_, i) => (
              <div key={i} className="card p-6">
                <Skeleton className="h-4 w-24 mb-3" />
                <Skeleton className="h-8 w-16" />
              </div>
            ))
          : statCards.map((stat) => {
              const Icon = stat.icon;
              return (
                <div key={stat.label} className="card p-6">
                  <div className="flex items-start justify-between">
                    <div>
                      <p className="text-text-tertiary text-sm font-medium">
                        {stat.label}
                      </p>
                      <p className="text-3xl font-bold text-text-primary mt-2">
                        {stat.value}
                      </p>
                    </div>
                    <div className={`p-3 rounded-lg ${stat.color}`}>
                      <Icon size={20} />
                    </div>
                  </div>
                </div>
              );
            })}
      </div>

      {/* Recent Documents Section */}
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <h2 className="section-header text-text-primary">
            Recent Documents
          </h2>
          <Link
            to="/recent"
            className="text-sm font-medium text-accent-primary hover:text-accent-hover transition-colors"
          >
            View All
          </Link>
        </div>

        {recentLoading ? (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
            {Array.from({ length: 4 }).map((_, i) => (
              <div key={i} className="card p-4">
                <div className="flex items-start gap-3">
                  <Skeleton className="h-10 w-10 flex-shrink-0" />
                  <div className="flex-1 space-y-2">
                    <Skeleton className="h-4 w-full" />
                    <Skeleton className="h-3 w-20" />
                  </div>
                </div>
              </div>
            ))}
          </div>
        ) : recentDocs && recentDocs.length > 0 ? (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
            {recentDocs.map((doc) => (
              <Link
                key={doc.id}
                to={`/document/${doc.id}`}
                className="card p-4 hover:shadow-md transition-shadow"
              >
                <div className="flex items-start gap-3">
                  <div className="p-2 rounded-lg bg-accent-subtle text-accent-primary flex-shrink-0">
                    <FileText size={20} />
                  </div>
                  <div className="flex-1 min-w-0">
                    <p className="font-medium text-text-primary text-sm truncate">
                      {doc.name}
                    </p>
                    <p className="text-xs text-text-tertiary mt-1">
                      {doc.docType.toUpperCase()}
                    </p>
                    <p className="text-xs text-text-tertiary mt-1">
                      {safeDistance(doc.modifiedAt)}
                    </p>
                  </div>
                </div>
              </Link>
            ))}
          </div>
        ) : (
          <div className="card p-8 text-center">
            <FolderPlus
              size={32}
              className="mx-auto text-text-tertiary mb-3"
            />
            <p className="text-text-secondary text-sm">
              No documents yet.{" "}
              <Link
                to="/watched"
                className="text-accent-primary hover:text-accent-hover"
              >
                Add a watched folder
              </Link>{" "}
              to get started.
            </p>
          </div>
        )}
      </div>

      {/* Top Spaces Section */}
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <h2 className="section-header text-text-primary">Top Spaces</h2>
          <Link
            to="/spaces"
            className="text-sm font-medium text-accent-primary hover:text-accent-hover transition-colors"
          >
            View All
          </Link>
        </div>

        {spacesLoading ? (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
            {Array.from({ length: 3 }).map((_, i) => (
              <div key={i} className="card p-6">
                <Skeleton className="h-10 w-10 mb-3" />
                <Skeleton className="h-5 w-24 mb-2" />
                <Skeleton className="h-3 w-16" />
              </div>
            ))}
          </div>
        ) : topSpaces.length > 0 ? (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
            {topSpaces.map((space) => (
              <Link
                key={space.id}
                to={`/spaces/${space.id}`}
                className="card p-6 hover:shadow-lg hover:border-accent-primary/50 transition-all border-l-4"
                style={{ borderLeftColor: space.color }}
              >
                <div className="space-y-3">
                  <div className="flex items-start justify-between">
                    <div
                      className="p-2 rounded-lg"
                      style={{
                        backgroundColor: `${space.color}15`,
                        color: space.color,
                      }}
                    >
                      <Brain size={24} />
                    </div>
                  </div>
                  <div>
                    <p className="font-semibold text-text-primary text-lg">
                      {space.name}
                    </p>
                    <p className="text-text-tertiary text-sm">
                      {space.documentCount} documents
                    </p>
                  </div>
                </div>
              </Link>
            ))}
          </div>
        ) : (
          <div className="card p-8 text-center">
            <Brain size={32} className="mx-auto text-text-tertiary mb-3" />
            <p className="text-text-secondary text-sm">
              No spaces yet. Spaces are auto-created as documents are indexed.
            </p>
          </div>
        )}
      </div>

      {/* Activity Timeline */}
      <div className="space-y-4">
        <h2 className="section-header text-text-primary">Activity</h2>
        <div className="card p-6">
          {activityLoading ? (
            <div className="space-y-4">
              {Array.from({ length: 3 }).map((_, i) => (
                <div key={i} className="flex items-center gap-3">
                  <Skeleton className="h-2 w-2 rounded-full" />
                  <Skeleton className="h-4 w-48" />
                </div>
              ))}
            </div>
          ) : activityItems && activityItems.length > 0 ? (
            <div className="space-y-4">
              {activityItems.map((item) => (
                <div key={item.id} className="flex items-center gap-3">
                  <div
                    className={`h-2 w-2 rounded-full flex-shrink-0 ${
                      item.type === "success"
                        ? "bg-success"
                        : item.type === "warning"
                          ? "bg-warning"
                          : item.type === "error"
                            ? "bg-error"
                            : "bg-info"
                    }`}
                  />
                  <p className="text-text-secondary text-sm flex-1">
                    {item.subject}
                  </p>
                  <span className="text-xs text-text-tertiary whitespace-nowrap">
                    {safeDistance(item.timestamp)}
                  </span>
                </div>
              ))}
            </div>
          ) : (
            <p className="text-text-tertiary text-sm">No recent activity.</p>
          )}
        </div>
      </div>
    </div>
  );
}
