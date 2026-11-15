#pragma once

#include <KAbstractFileItemActionPlugin>


class DaemonClient;

class RcloneFileItemAction : public KAbstractFileItemActionPlugin
{
    Q_OBJECT
public:
    RcloneFileItemAction(QObject *parent, const QVariantList &args);

    QList<QAction *> actions(const KFileItemListProperties &fileItemInfos, QWidget *parentWidget) override;

private:
    DaemonClient *m_client;
};
