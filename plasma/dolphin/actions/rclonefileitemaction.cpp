#include "rclonefileitemaction.h"
#include "daemonclient.h"

#include <KFileItem>
#include <KFileItemListProperties>
#ifdef RCLONE_NO_KI18N
#include <QCoreApplication>
#define i18n(s) QCoreApplication::translate("rclone-plasma", s)
#else
#include <KLocalizedString>
#endif
#include <KPluginFactory>

#include <QAction>
#include <QIcon>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMenu>
#include <QProcess>
#include <QUrl>

K_PLUGIN_CLASS_WITH_JSON(RcloneFileItemAction, "rclonefileitemaction.json")

namespace
{
void launch(const QStringList &args)
{
    QProcess::startDetached(QStringLiteral("rclone-plasma"), args);
}
}

RcloneFileItemAction::RcloneFileItemAction(QObject *parent, const QVariantList &)
    : KAbstractFileItemActionPlugin(parent)
    , m_client(new DaemonClient(this))
{
    // Pins arrive asynchronously; they are consulted when the menu is built.
    connect(m_client, &DaemonClient::mountPathChanged, this, &RcloneFileItemAction::refreshPins);
    refreshPins();
}

void RcloneFileItemAction::refreshPins()
{
    m_client->fetchStatusJson([this](const QString &json) {
        const QJsonObject offline = QJsonDocument::fromJson(json.toUtf8()).object().value(QLatin1String("offline")).toObject();
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
        m_pins = pins;
        m_pinsKnown = true;
    });
}

QList<QAction *> RcloneFileItemAction::actions(const KFileItemListProperties &fileItemInfos, QWidget *parentWidget)
{
    // Kick off a refresh for the next invocation; never wait for it.
    if (m_client->mountPath().isEmpty()) {
        m_client->refreshMountPath();
        return {};
    }
    refreshPins();

    const KFileItemList items = fileItemInfos.items();
    if (items.isEmpty()) {
        return {};
    }
    QStringList paths;
    for (const KFileItem &item : items) {
        const QUrl url = item.url();
        if (!url.isLocalFile() || !m_client->isUnderMount(url.toLocalFile())) {
            return {};
        }
        paths << url.toLocalFile();
    }

    auto *menuAction = new QAction(QIcon::fromTheme(QStringLiteral("rclone-plasma")), i18n("rclone"), parentWidget);
    auto *menu = new QMenu(parentWidget);
    menuAction->setMenu(menu);

    QAction *details = menu->addAction(QIcon::fromTheme(QStringLiteral("document-properties")), i18n("Details…"));
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
            const bool pinned = m_pins.contains(rel);
            if (!m_pinsKnown || !pinned) {
                QAction *pin = menu->addAction(QIcon::fromTheme(QStringLiteral("emblem-favorite")), i18n("Keep offline"));
                connect(pin, &QAction::triggered, this, [path]() {
                    launch({QStringLiteral("--pin"), path});
                });
            }
            if (!m_pinsKnown || pinned) {
                QAction *unpin = menu->addAction(QIcon::fromTheme(QStringLiteral("edit-delete")), i18n("Stop keeping offline"));
                connect(unpin, &QAction::triggered, this, [path]() {
                    launch({QStringLiteral("--unpin"), path});
                });
            }
        }
        QAction *share = menu->addAction(QIcon::fromTheme(QStringLiteral("emblem-shared")), i18n("Manage Sharing"));
        connect(share, &QAction::triggered, this, [path]() {
            launch({QStringLiteral("--share-open"), path});
        });
    }

    return {menuAction};
}

#include "rclonefileitemaction.moc"
