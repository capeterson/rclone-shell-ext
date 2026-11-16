#include "rclonefileitemaction.h"
#include "daemonclient.h"

#include <KFileItem>
#include <KFileItemListProperties>
#include <KPluginFactory>

#include <QAction>
#include <QIcon>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMenu>
#include <QProcess>
#include <QSet>
#include <QUrl>

K_PLUGIN_CLASS_WITH_JSON(RcloneFileItemAction, "rclonefileitemaction.json")

namespace
{
void launch(const QStringList &args)
{
    QProcess::startDetached(QStringLiteral("rclone-shell-ext-plasma"), args);
}

// Dolphin creates a fresh KFileItemActions (and so a fresh instance of this plugin) for every
// context menu, and actions() is called right after construction. Async fetches would never
// land in time, so the daemon is queried synchronously with a short timeout instead.
constexpr int kDbusTimeoutMs = 300;

QSet<QString> parsePins(const QString &statusJson)
{
    const QJsonObject offline = QJsonDocument::fromJson(statusJson.toUtf8()).object().value(QLatin1String("offline")).toObject();
    QSet<QString> pins;
    const auto arr = offline.value(QLatin1String("pins")).toArray();
    for (const auto &v : arr) {
        QString rel = v.toObject().value(QLatin1String("rel")).toString();
        while (rel.startsWith(QLatin1Char('/'))) {
            rel.remove(0, 1);
        }
        while (rel.endsWith(QLatin1Char('/'))) {
            rel.chop(1);
        }
        pins.insert(rel);
    }
    return pins;
}
}

RcloneFileItemAction::RcloneFileItemAction(QObject *parent, const QVariantList &)
    : KAbstractFileItemActionPlugin(parent)
    , m_client(new DaemonClient(this))
{
}

QList<QAction *> RcloneFileItemAction::actions(const KFileItemListProperties &fileItemInfos, QWidget *parentWidget)
{
    const KFileItemList items = fileItemInfos.items();
    if (items.isEmpty()) {
        return {};
    }
    // Cheap pre-filter before touching D-Bus: only local files can be under the mount.
    for (const KFileItem &item : items) {
        if (!item.url().isLocalFile()) {
            return {};
        }
    }
    if (m_client->ensureMountPath(kDbusTimeoutMs).isEmpty()) {
        return {};
    }
    QStringList paths;
    for (const KFileItem &item : items) {
        const QString path = item.url().toLocalFile();
        if (!m_client->isUnderMount(path)) {
            return {};
        }
        paths << path;
    }

    auto *menuAction = new QAction(QIcon::fromTheme(QStringLiteral("com.ca-peterson.rclone-shell-ext")), m_client->text("dolphin-menu"), parentWidget);
    auto *menu = new QMenu(parentWidget);
    menuAction->setMenu(menu);

    QAction *details = menu->addAction(QIcon::fromTheme(QStringLiteral("document-properties")), m_client->text("dolphin-details"));
    connect(details, &QAction::triggered, this, [paths]() {
        launch(QStringList{QStringLiteral("--menu")} + paths);
    });

    if (paths.size() == 1) {
        const QString path = paths.first();
        if (items.first().isDir()) {
            QString rel = path.mid(m_client->mountPath().size());
            while (rel.startsWith(QLatin1Char('/'))) {
                rel.remove(0, 1);
            }
            const QString json = m_client->getPropertyBlocking(QStringLiteral("StatusJson"), kDbusTimeoutMs);
            const bool pinsKnown = !json.isEmpty();
            const bool pinned = pinsKnown && parsePins(json).contains(rel);
            if (!pinsKnown || !pinned) {
                QAction *pin = menu->addAction(QIcon::fromTheme(QStringLiteral("emblem-favorite")), m_client->text("details-keep-offline"));
                connect(pin, &QAction::triggered, this, [path]() {
                    launch({QStringLiteral("--pin"), path});
                });
            }
            if (!pinsKnown || pinned) {
                QAction *unpin = menu->addAction(QIcon::fromTheme(QStringLiteral("edit-delete")), m_client->text("dolphin-stop-keeping-offline"));
                connect(unpin, &QAction::triggered, this, [path]() {
                    launch({QStringLiteral("--unpin"), path});
                });
            }
        }
        QAction *share = menu->addAction(QIcon::fromTheme(QStringLiteral("emblem-shared")), m_client->text("details-manage-sharing"));
        connect(share, &QAction::triggered, this, [path]() {
            launch({QStringLiteral("--share-open"), path});
        });
    }

    return {menuAction};
}

#include "rclonefileitemaction.moc"
