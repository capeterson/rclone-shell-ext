#pragma once

#include <KAbstractFileItemActionPlugin>


class DaemonClient;
class KFileItem;
class QMenu;

class RcloneFileItemAction : public KAbstractFileItemActionPlugin
{
    Q_OBJECT
public:
    RcloneFileItemAction(QObject *parent, const QVariantList &args);

    QList<QAction *> actions(const KFileItemListProperties &fileItemInfos, QWidget *parentWidget) override;

private:
    /// The "rclone" submenu action; its (empty) menu is returned in `menu`.
    QAction *submenu(QWidget *parentWidget, QMenu **menu);
    /// The actions for a selection outside the mount: "Back up to Drive" or "Stop backing up"
    /// for a single folder, nothing otherwise.
    QList<QAction *> backupActions(const KFileItem &item, qsizetype count, QWidget *parentWidget);

    DaemonClient *m_client;
};
