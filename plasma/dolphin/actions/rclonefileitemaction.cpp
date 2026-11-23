#include "rclonefileitemaction.h"
#include "daemonclient.h"

#include <KFileItem>
#include <KFileItemListProperties>
#include <KPluginFactory>

#include <QAction>
#include <QDir>
#include <QFileInfo>
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

// Is `path` itself one of the backed-up folders in the GetConfig JSON? The config keeps sources
// absolute with symlinks resolved, so the folder is compared both as shown and resolved. A folder
// inside a backed-up one is not a match: removing it would fail, and adding it is refused by the
// CLI with a notification that says why.
bool isBackedUp(const QString &configJson, const QString &path)
{
    const QString shown = QDir::cleanPath(path);
    const QString resolved = QFileInfo(path).canonicalFilePath();
    const auto folders = QJsonDocument::fromJson(configJson.toUtf8()).object().value(QLatin1String("backup_folders")).toArray();
    for (const auto &v : folders) {
        const QString source = QDir::cleanPath(v.toObject().value(QLatin1String("source")).toString());
        if (!source.isEmpty() && (source == shown || source == resolved)) {
            return true;
        }
    }
    return false;
}
}

RcloneFileItemAction::RcloneFileItemAction(QObject *parent, const QVariantList &)
    : KAbstractFileItemActionPlugin(parent)
    , m_client(new DaemonClient(this))
{
}

QAction *RcloneFileItemAction::submenu(QWidget *parentWidget, QMenu **menu)
{
    auto *menuAction = new QAction(QIcon::fromTheme(QStringLiteral("com.ca-peterson.rclone-shell-ext")), m_client->text("dolphin-menu"), parentWidget);
    *menu = new QMenu(parentWidget);
    menuAction->setMenu(*menu);
    return menuAction;
}

QList<QAction *> RcloneFileItemAction::actions(const KFileItemListProperties &fileItemInfos, QWidget *parentWidget)
{
    const KFileItemList items = fileItemInfos.items();
    if (items.isEmpty()) {
        return {};
    }
    // Cheap pre-filter before touching D-Bus: only local files can be under the mount or backed up.
    for (const KFileItem &item : items) {
        if (!item.url().isLocalFile()) {
            return {};
        }
    }
    // Without a daemon the mount path is unknown and everything counts as outside it; the CLI
    // refuses to back up a folder in the mount.
    m_client->ensureMountPath(kDbusTimeoutMs);
    QStringList paths;
    bool inside = false;
    bool outside = false;
    for (const KFileItem &item : items) {
        const QString path = item.url().toLocalFile();
        (m_client->isUnderMount(path) ? inside : outside) = true;
        paths << path;
    }
    if (inside && outside) {
        return {};
    }
    if (outside) {
        return backupActions(items.first(), items.size(), parentWidget);
    }

    QMenu *menu = nullptr;
    QAction *menuAction = submenu(parentWidget, &menu);

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
                QAction *unpin = menu->addAction(QIcon::fromTheme(QStringLiteral("edit-delete")), m_client->text("details-stop-keeping-offline"));
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

QList<QAction *> RcloneFileItemAction::backupActions(const KFileItem &item, qsizetype count, QWidget *parentWidget)
{
    // Outside the mount only a single folder has an action: backing it up (or stopping).
    if (count != 1 || !item.isDir()) {
        return {};
    }
    const QString path = item.url().toLocalFile();
    // An empty reply (no daemon) offers "Back up to Drive"; the CLI says if it already is.
    const bool backedUp = isBackedUp(m_client->callStringBlocking(QStringLiteral("GetConfig"), kDbusTimeoutMs), path);

    QMenu *menu = nullptr;
    QAction *menuAction = submenu(parentWidget, &menu);
    if (backedUp) {
        QAction *remove = menu->addAction(QIcon::fromTheme(QStringLiteral("list-remove")), m_client->text("backup-menu-remove"));
        connect(remove, &QAction::triggered, this, [path]() {
            launch({QStringLiteral("--backup-remove"), path});
        });
    } else {
        QAction *add = menu->addAction(QIcon::fromTheme(QStringLiteral("folder-cloud")), m_client->text("backup-menu-add"));
        connect(add, &QAction::triggered, this, [path]() {
            launch({QStringLiteral("--backup"), path});
        });
    }
    return {menuAction};
}

#include "rclonefileitemaction.moc"
