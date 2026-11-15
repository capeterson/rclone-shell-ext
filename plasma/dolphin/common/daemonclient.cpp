#include "daemonclient.h"

#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusMetaType>
#include <QDBusPendingCallWatcher>
#include <QDBusPendingReply>
#include <QDBusVariant>

namespace
{
const QString kService = QStringLiteral("com.ca-peterson.rclone-shell-ext");
const QString kPath = QStringLiteral("/com/ca_peterson/rclone_shell_ext");
const QString kIface = QStringLiteral("com.ca_peterson.RcloneShellExt1");
constexpr qint64 kBackoffMs = 10000;
}

DaemonClient::DaemonClient(QObject *parent)
    : QObject(parent)
    , m_watcher(kService, QDBusConnection::sessionBus(), QDBusServiceWatcher::WatchForOwnerChange)
{
    qDBusRegisterMetaType<StatesMap>();

    auto bus = QDBusConnection::sessionBus();
    bus.connect(kService, kPath, kIface, QStringLiteral("ItemsChanged"), this, SIGNAL(itemsChanged(QStringList)));

    connect(&m_watcher, &QDBusServiceWatcher::serviceOwnerChanged, this, [this](const QString &, const QString &, const QString &newOwner) {
        if (newOwner.isEmpty()) {
            return;
        }
        // Daemon (re)started: the old error state no longer applies.
        m_hadError = false;
        refreshMountPath();
    });

    refreshMountPath();
}

bool DaemonClient::isUnderMount(const QString &path) const
{
    if (m_mountPath.isEmpty()) {
        return false;
    }
    return path == m_mountPath || path.startsWith(m_mountPath + QLatin1Char('/'));
}

bool DaemonClient::backedOff() const
{
    return m_hadError && m_errorTimer.isValid() && m_errorTimer.elapsed() < kBackoffMs;
}

void DaemonClient::noteError()
{
    m_hadError = true;
    m_errorTimer.restart();
}

void DaemonClient::getProperty(const QString &name, std::function<void(const QString &)> cb)
{
    if (backedOff()) {
        return;
    }
    auto msg = QDBusMessage::createMethodCall(kService, kPath, QStringLiteral("org.freedesktop.DBus.Properties"), QStringLiteral("Get"));
    msg << kIface << name;
    auto *w = new QDBusPendingCallWatcher(QDBusConnection::sessionBus().asyncCall(msg), this);
    connect(w, &QDBusPendingCallWatcher::finished, this, [this, w, cb = std::move(cb)]() {
        w->deleteLater();
        QDBusPendingReply<QDBusVariant> reply = *w;
        if (reply.isError()) {
            noteError();
            return;
        }
        cb(reply.value().variant().toString());
    });
}

QString DaemonClient::getPropertyBlocking(const QString &name, int timeoutMs)
{
    if (backedOff()) {
        return {};
    }
    auto msg = QDBusMessage::createMethodCall(kService, kPath, QStringLiteral("org.freedesktop.DBus.Properties"), QStringLiteral("Get"));
    msg << kIface << name;
    const QDBusMessage reply = QDBusConnection::sessionBus().call(msg, QDBus::Block, timeoutMs);
    if (reply.type() != QDBusMessage::ReplyMessage || reply.arguments().isEmpty()) {
        noteError();
        return {};
    }
    return reply.arguments().first().value<QDBusVariant>().variant().toString();
}

void DaemonClient::refreshMountPath()
{
    getProperty(QStringLiteral("MountPath"), [this](const QString &p) {
        setMountPath(p);
    });
}

QString DaemonClient::ensureMountPath(int timeoutMs)
{
    if (m_mountPath.isEmpty()) {
        const QString p = getPropertyBlocking(QStringLiteral("MountPath"), timeoutMs);
        if (!p.isEmpty()) {
            setMountPath(p);
        }
    }
    return m_mountPath;
}

void DaemonClient::setMountPath(const QString &raw)
{
    QString path = raw;
    while (path.size() > 1 && path.endsWith(QLatin1Char('/'))) {
        path.chop(1);
    }
    if (path != m_mountPath) {
        m_mountPath = path;
        Q_EMIT mountPathChanged(m_mountPath);
    }
}

void DaemonClient::fetchStatusJson(std::function<void(const QString &)> cb)
{
    getProperty(QStringLiteral("StatusJson"), std::move(cb));
}

void DaemonClient::fileStates(const QStringList &paths, std::function<void(const StatesMap &)> cb)
{
    if (backedOff() || paths.isEmpty()) {
        return;
    }
    auto msg = QDBusMessage::createMethodCall(kService, kPath, kIface, QStringLiteral("FileStates"));
    msg << QVariant::fromValue(paths);
    auto *w = new QDBusPendingCallWatcher(QDBusConnection::sessionBus().asyncCall(msg), this);
    connect(w, &QDBusPendingCallWatcher::finished, this, [this, w, cb = std::move(cb)]() {
        w->deleteLater();
        QDBusPendingReply<StatesMap> reply = *w;
        if (reply.isError()) {
            noteError();
            return;
        }
        cb(reply.value());
    });
}
