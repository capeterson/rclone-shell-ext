#pragma once

#include <QDBusServiceWatcher>
#include <QElapsedTimer>
#include <QMap>
#include <QObject>
#include <QString>
#include <QStringList>
#include <functional>

/**
 * Async wrapper around the rclone-shell-ext-plasma daemon's session-bus interface.
 * Never blocks: every call is a QDBusPendingCall. After an error, further calls
 * are suppressed for 10 s so a missing daemon is not hammered (or autostart-spammed).
 */
class DaemonClient : public QObject
{
    Q_OBJECT
public:
    using StatesMap = QMap<QString, QStringList>;

    explicit DaemonClient(QObject *parent = nullptr);

    /// Cached mount path; empty until known.
    QString mountPath() const { return m_mountPath; }
    /// True if path is the mount or below it (requires a known mount path).
    bool isUnderMount(const QString &path) const;

    /// Async FileStates call. The callback runs only on success.
    void fileStates(const QStringList &paths, std::function<void(const StatesMap &)> cb);
    /// Async StatusJson fetch. The callback runs only on success.
    void fetchStatusJson(std::function<void(const QString &)> cb);
    /// Re-read MountPath asynchronously.
    void refreshMountPath();

    /// Blocking property read with a short timeout, for callers that must answer synchronously
    /// (e.g. a context-menu plugin instantiated per menu). Empty on error/timeout.
    QString getPropertyBlocking(const QString &name, int timeoutMs);
    /// If the mount path is unknown, fetch it with a blocking call. Returns mountPath().
    QString ensureMountPath(int timeoutMs);

Q_SIGNALS:
    void mountPathChanged(const QString &mountPath);
    void itemsChanged(const QStringList &paths);

private:
    bool backedOff() const;
    void noteError();
    void getProperty(const QString &name, std::function<void(const QString &)> cb);
    void setMountPath(const QString &raw);

    QString m_mountPath;
    QDBusServiceWatcher m_watcher;
    QElapsedTimer m_errorTimer;
    bool m_hadError = false;
};
