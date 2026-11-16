#include "rcloneoverlayplugin.h"
#include "daemonclient.h"

#include <QUrl>

namespace
{
constexpr int kMaxCache = 5000;
constexpr int kBatchDelayMs = 50;
}

RcloneOverlayPlugin::RcloneOverlayPlugin(QObject *parent)
    : KOverlayIconPlugin(parent)
    , m_client(new DaemonClient(this))
{
    // The overlay plugin lives as long as Dolphin; the per-menu action plugin then finds the
    // UI text already cached.
    DaemonClient::prefetchStrings();
    m_flushTimer.setSingleShot(true);
    m_flushTimer.setInterval(kBatchDelayMs);
    connect(&m_flushTimer, &QTimer::timeout, this, &RcloneOverlayPlugin::flush);

    connect(m_client, &DaemonClient::mountPathChanged, this, [this]() {
        m_cache.clear();
    });

    connect(m_client, &DaemonClient::itemsChanged, this, [this](const QStringList &paths) {
        // Only cached paths are (probably) visible. Keep the old value until the
        // reply arrives so we can tell whether anything really changed.
        for (const QString &p : paths) {
            if (m_cache.contains(p)) {
                queue(p);
            }
        }
    });
}

QStringList RcloneOverlayPlugin::getOverlays(const QUrl &item)
{
    if (!item.isLocalFile()) {
        return {};
    }
    const QString path = item.toLocalFile();
    if (!m_client->isUnderMount(path)) {
        return {};
    }
    const auto it = m_cache.constFind(path);
    if (it != m_cache.constEnd()) {
        return it.value();
    }
    queue(path);
    return {};
}

void RcloneOverlayPlugin::queue(const QString &path)
{
    if (m_inFlight.contains(path)) {
        return;
    }
    m_pending.insert(path);
    if (!m_flushTimer.isActive()) {
        m_flushTimer.start();
    }
}

void RcloneOverlayPlugin::flush()
{
    if (m_pending.isEmpty()) {
        return;
    }
    const QStringList paths(m_pending.cbegin(), m_pending.cend());
    m_pending.clear();
    m_inFlight.unite(QSet<QString>(paths.cbegin(), paths.cend()));

    m_client->fileStates(paths, [this, paths](const QMap<QString, QStringList> &states) {
        for (const QString &p : paths) {
            m_inFlight.remove(p);
        }
        onReply(states);
    });
    // On error the callback never runs; drop in-flight marks via a short deferred
    // cleanup so that a later getOverlays() can retry (after the client's backoff).
    QTimer::singleShot(5000, this, [this, paths]() {
        for (const QString &p : paths) {
            m_inFlight.remove(p);
        }
    });
}

void RcloneOverlayPlugin::onReply(const QMap<QString, QStringList> &states)
{
    for (auto it = states.cbegin(); it != states.cend(); ++it) {
        store(it.key(), it.value());
    }
}

void RcloneOverlayPlugin::store(const QString &path, const QStringList &emblems)
{
    if (m_cache.size() >= kMaxCache && !m_cache.contains(path)) {
        m_cache.clear(); // simple clear-on-overflow
    }
    const auto old = m_cache.constFind(path);
    const bool changed = (old == m_cache.constEnd()) ? !emblems.isEmpty() : (old.value() != emblems);
    m_cache.insert(path, emblems);
    if (changed) {
        Q_EMIT overlaysChanged(QUrl::fromLocalFile(path), emblems);
    }
}
