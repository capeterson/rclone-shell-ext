#include "daemonclient.h"

#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusMetaType>
#include <QDBusPendingCallWatcher>
#include <QDBusPendingReply>
#include <QDBusReply>
#include <QDBusVariant>
#include <QElapsedTimer>

namespace
{
const QString kService = QStringLiteral("com.ca-peterson.rclone-shell-ext");
const QString kPath = QStringLiteral("/com/ca_peterson/rclone_shell_ext");
const QString kIface = QStringLiteral("com.ca_peterson.RcloneShellExt1");
constexpr qint64 kBackoffMs = 10000;
constexpr int kStringsTimeoutMs = 300;
// Dolphin builds a new plugin (and so a new DaemonClient) for every context menu, so the UI text
// is cached for the whole process.
DaemonClient::StringMap s_strings;
// A blocking Strings() call is tried at most once per kBackoffMs per process, however many menus
// (and DaemonClient instances) come and go while the daemon is away.
QElapsedTimer s_stringsAttempt;
bool s_stringsPrefetching = false;
}

void DaemonClient::prefetchStrings()
{
    if (!s_strings.isEmpty() || s_stringsPrefetching) {
        return;
    }
    s_stringsPrefetching = true;
    auto msg = QDBusMessage::createMethodCall(kService, kPath, kIface, QStringLiteral("Strings"));
    // No parent: the call must outlive whichever client started it.
    auto *w = new QDBusPendingCallWatcher(QDBusConnection::sessionBus().asyncCall(msg));
    QObject::connect(w, &QDBusPendingCallWatcher::finished, w, [w]() {
        QDBusPendingReply<StringMap> reply = *w;
        if (!reply.isError()) {
            s_strings = reply.value();
        }
        s_stringsPrefetching = false;
        w->deleteLater();
    });
}

DaemonClient::DaemonClient(QObject *parent)
    : QObject(parent)
    , m_watcher(kService, QDBusConnection::sessionBus(), QDBusServiceWatcher::WatchForOwnerChange)
{
    qDBusRegisterMetaType<StatesMap>();
    qDBusRegisterMetaType<StringMap>();

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

QString DaemonClient::text(const QString &id)
{
    const bool mayBlock = !s_stringsAttempt.isValid() || s_stringsAttempt.elapsed() >= kBackoffMs;
    if (s_strings.isEmpty() && mayBlock) {
        s_stringsAttempt.restart();
        auto msg = QDBusMessage::createMethodCall(kService, kPath, kIface, QStringLiteral("Strings"));
        const QDBusReply<StringMap> reply = QDBusConnection::sessionBus().call(msg, QDBus::Block, kStringsTimeoutMs);
        if (reply.isValid()) {
            s_strings = reply.value();
        }
    }
    return s_strings.value(id, id);
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
